//! Long polling that repairs updates teloxide cannot read.
//!
//! teloxide-core 0.13 cannot parse an `external_reply` to a plain text
//! message: its `ExternalReplyInfoKind` has a variant for every kind of media
//! but none for text. Telegram sends one when someone replies to a message from
//! another chat, including a message sent before the group became a
//! supergroup. The whole message then silently degrades to an empty one, so
//! its text is neither moderated nor read as a command.
//!
//! Updates are fetched here as raw JSON, an unreadable `external_reply` is
//! dropped, and only then are they parsed. Everything else works like
//! teloxide's own polling.

use std::collections::VecDeque;
use std::time::Duration;

use futures::stream::{self, BoxStream, StreamExt};
use serde::Serialize;
use serde_json::Value;
use teloxide::errors::AsResponseParameters;
use teloxide::prelude::*;
use teloxide::requests::{JsonRequest, Payload};
use teloxide::stop::{mk_stop_token, StopFlag, StopToken};
use teloxide::types::{AllowedUpdate, ExternalReplyInfo, MessageKind, UpdateKind};
use teloxide::update_listeners::{AsUpdateStream, UpdateListener};
use teloxide::RequestError;

/// How long Telegram holds a `getUpdates` call open waiting for updates.
const POLL_TIMEOUT_SECS: u32 = 10;
/// Pause after a failed `getUpdates` call, unless Telegram names its own.
const RETRY_DELAY: Duration = Duration::from_secs(5);

/// `getUpdates` that returns raw JSON instead of parsed updates.
#[derive(Serialize)]
struct GetRawUpdates {
    offset: i32,
    timeout: u32,
    #[serde(skip_serializing_if = "Option::is_none")]
    limit: Option<u8>,
    #[serde(skip_serializing_if = "Option::is_none")]
    allowed_updates: Option<Vec<AllowedUpdate>>,
}

impl Payload for GetRawUpdates {
    type Output = Vec<Value>;
    const NAME: &'static str = "GetUpdates";

    fn timeout_hint(&self) -> Option<Duration> {
        Some(Duration::from_secs(self.timeout.into()))
    }
}

/// The update listener the dispatcher runs on.
pub struct Poller {
    bot: Bot,
    /// The next update to ask for; confirms everything before it.
    offset: i32,
    /// Sent with the first call only; Telegram remembers it.
    allowed_updates: Option<Vec<AllowedUpdate>>,
    token: StopToken,
    flag: StopFlag,
}

impl Poller {
    /// Drop a webhook left over from another deployment (Telegram refuses
    /// `getUpdates` while one is set) and get ready to poll.
    pub async fn new(bot: Bot) -> Poller {
        if let Err(e) = bot.delete_webhook().await {
            tracing::warn!(error = %e, "delete_webhook_failed");
        }
        let (token, flag) = mk_stop_token();
        Poller {
            bot,
            offset: 0,
            allowed_updates: None,
            token,
            flag,
        }
    }

    fn fetch(&mut self, timeout: u32, limit: Option<u8>) -> GetRawUpdatesRequest {
        let payload = GetRawUpdates {
            offset: self.offset,
            timeout,
            limit,
            allowed_updates: self.allowed_updates.take(),
        };
        JsonRequest::new(self.bot.clone(), payload).send()
    }

    /// Parse a batch, moving the offset past every update in it, readable or not.
    fn accept(&mut self, raw: Vec<Value>) -> Vec<Update> {
        raw.into_iter()
            .filter_map(|mut value| {
                if let Some(id) = value.get("update_id").and_then(Value::as_i64) {
                    self.offset = self.offset.max(id as i32 + 1);
                }
                parse(&mut value)
            })
            .collect()
    }

    /// Tell Telegram the last batch was handled, so a restart does not
    /// replay it.
    async fn confirm(&mut self) {
        if let Err(e) = self.fetch(0, Some(1)).await {
            tracing::debug!(error = %e, "confirm_updates_failed");
        }
    }
}

type GetRawUpdatesRequest = <JsonRequest<GetRawUpdates> as Request>::Send;

impl UpdateListener for Poller {
    type Err = RequestError;

    fn stop_token(&mut self) -> StopToken {
        self.token.clone()
    }

    fn hint_allowed_updates(&mut self, hint: &mut dyn Iterator<Item = AllowedUpdate>) {
        self.allowed_updates = Some(hint.collect());
    }
}

impl<'a> AsUpdateStream<'a> for Poller {
    type StreamErr = RequestError;
    type Stream = BoxStream<'a, Result<Update, RequestError>>;

    fn as_stream(&'a mut self) -> Self::Stream {
        stream::unfold((self, VecDeque::new()), |(poller, mut buffer)| async move {
            loop {
                if let Some(update) = buffer.pop_front() {
                    return Some((Ok(update), (poller, buffer)));
                }
                let stop = poller.flag.clone();
                if stop.is_stopped() {
                    poller.confirm().await;
                    return None;
                }
                let request = poller.fetch(POLL_TIMEOUT_SECS, None);
                let result = tokio::select! {
                    result = request => result,
                    () = stop.clone() => {
                        poller.confirm().await;
                        return None;
                    }
                };
                match result {
                    Ok(raw) => buffer.extend(poller.accept(raw)),
                    Err(e) => {
                        let delay = e.retry_after().map_or(RETRY_DELAY, |s| s.duration());
                        tokio::select! {
                            () = tokio::time::sleep(delay) => {}
                            () = stop => {}
                        }
                        // The dispatcher's error handler logs it.
                        return Some((Err(e), (poller, buffer)));
                    }
                }
            }
        })
        .boxed()
    }
}

/// Repair and parse one raw update. `None` only for JSON that is not an
/// update at all.
fn parse(value: &mut Value) -> Option<Update> {
    let repaired = repair(value);
    let had_text = mentions_text(value);
    // Not `from_value`: teloxide's `Update` only deserializes from text and
    // turns anything else into `UpdateKind::Error`.
    match serde_json::from_str::<Update>(&value.to_string()) {
        Ok(update) => {
            if repaired > 0 {
                tracing::debug!(update = update.id.0, repaired, "update_repaired");
            }
            if had_text && is_empty_message(&update) {
                // Some other field teloxide cannot read; the text is lost.
                tracing::warn!(update = update.id.0, "unreadable_message");
            }
            Some(update)
        }
        Err(e) => {
            tracing::warn!(error = %e, "unparsable_update");
            None
        }
    }
}

/// Drop every `external_reply` teloxide cannot parse; returns how many.
fn repair(value: &mut Value) -> usize {
    match value {
        Value::Object(map) => {
            let unreadable = map
                .get("external_reply")
                .is_some_and(|r| serde_json::from_value::<ExternalReplyInfo>(r.clone()).is_err());
            if unreadable {
                map.remove("external_reply");
            }
            usize::from(unreadable) + map.values_mut().map(repair).sum::<usize>()
        }
        Value::Array(items) => items.iter_mut().map(repair).sum(),
        _ => 0,
    }
}

/// Whether the update's message carries text or a caption in the raw JSON.
fn mentions_text(value: &Value) -> bool {
    ["message", "edited_message"].iter().any(|key| {
        value
            .get(key)
            .is_some_and(|m| m.get("text").is_some() || m.get("caption").is_some())
    })
}

fn is_empty_message(update: &Update) -> bool {
    matches!(
        &update.kind,
        UpdateKind::Message(m) | UpdateKind::EditedMessage(m) if matches!(m.kind, MessageKind::Empty {})
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A reply to a text message from another chat, as Telegram sends it.
    fn command_replying_elsewhere() -> Value {
        serde_json::json!({
            "update_id": 41,
            "message": {
                "message_id": 10,
                "date": 1761335100,
                "chat": {"id": -1004302764108_i64, "type": "supergroup", "title": "t"},
                "from": {"id": 223217484, "is_bot": false, "first_name": "A"},
                "text": "/spam",
                "entities": [{"type": "bot_command", "offset": 0, "length": 5}],
                "external_reply": {
                    "origin": {
                        "type": "user",
                        "date": 1761334320,
                        "sender_user": {"id": 2098633721, "is_bot": false, "first_name": "M"}
                    },
                    "chat": {"id": -5500889472_i64, "type": "group", "title": "t"},
                    "message_id": 3
                }
            }
        })
    }

    fn message_text(update: &Update) -> Option<&str> {
        match &update.kind {
            UpdateKind::Message(m) => m.text(),
            _ => None,
        }
    }

    #[test]
    fn teloxide_alone_loses_the_text() {
        let update: Update =
            serde_json::from_str(&command_replying_elsewhere().to_string()).unwrap();
        assert_eq!(
            message_text(&update),
            None,
            "fixed upstream? drop the repair"
        );
    }

    #[test]
    fn repaired_update_keeps_the_text() {
        let mut raw = command_replying_elsewhere();

        let update = parse(&mut raw).unwrap();

        assert_eq!(message_text(&update), Some("/spam"));
        assert!(raw["message"].get("external_reply").is_none());
    }

    #[test]
    fn readable_external_replies_are_kept() {
        let mut raw = command_replying_elsewhere();
        raw["message"]["external_reply"]["dice"] = serde_json::json!({"emoji": "🎲", "value": 3});

        assert_eq!(repair(&mut raw), 0);
        assert!(raw["message"].get("external_reply").is_some());
    }

    #[test]
    fn offset_moves_past_every_update() {
        let mut poller = Poller {
            bot: Bot::new("1:test"),
            offset: 0,
            allowed_updates: None,
            token: mk_stop_token().0,
            flag: mk_stop_token().1,
        };

        let updates = poller.accept(vec![
            command_replying_elsewhere(),
            serde_json::json!({"update_id": 42, "something_new": {}}),
            serde_json::json!({"no_update_id": true}),
        ]);

        assert_eq!(updates.len(), 2);
        assert_eq!(poller.offset, 43);
    }
}
