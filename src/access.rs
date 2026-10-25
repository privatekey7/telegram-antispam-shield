//! Who may make the bot act: the rights each admin action needs, checked
//! without Telegram so the policy can be unit-tested (ADR 0003).
//!
//! The bot has the rights it was given in a group, so it must not do for a
//! member what that member could not do alone (confused deputy):
//!
//! * banning actions (`/spam`, `/ban`, the report's "Ban" button) need the
//!   "Ban users" right (`can_restrict_members`) and, since they also delete
//!   messages and `/spam` teaches the bot to delete similar ones, "Delete
//!   messages" (`can_delete_messages`);
//! * "Not spam" unbans, so it needs "Ban users";
//! * `/warn` deletes, so it needs "Delete messages", and also "Ban users"
//!   while warnings can end in a ban (`ANTISPAM_WARN_LIMIT` > 0);
//! * the chat creator has every right, and the bot's owner may do anything;
//! * anonymous admins (posting as the group) are refused: Telegram does not
//!   say which admin is behind the message, so their rights are unknown.
//!
//! Commands also refuse some targets: admins and the bot's owner (Telegram
//! would not ban an admin, yet warnings would pile up), and posts of the group
//! itself (anonymous admins, the linked channel).

/// The rights of one administrator that matter to the bot.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub struct AdminRights {
    /// "Delete messages" (`can_delete_messages`).
    pub delete_messages: bool,
    /// "Ban users" (`can_restrict_members`): ban, unban, mute, unmute.
    pub restrict_members: bool,
}

impl AdminRights {
    /// The chat creator has every right.
    pub const CREATOR: AdminRights = AdminRights {
        delete_messages: true,
        restrict_members: true,
    };

    /// Whether these rights include `privilege`.
    pub fn allow(self, privilege: Privilege) -> bool {
        match privilege {
            Privilege::DeleteMessages => self.delete_messages,
            Privilege::RestrictMembers => self.restrict_members,
        }
    }
}

/// A Telegram admin right an action needs.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Privilege {
    DeleteMessages,
    RestrictMembers,
}

/// Something an admin asks the bot to do.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum AdminAction {
    /// `/spam`: delete, ban and learn.
    Spam,
    /// `/ban`: delete and ban.
    Ban,
    /// `/warn`: delete and warn; enough warnings mean a ban.
    Warn,
    /// The report's "Not spam" button: unban or unmute, trust, learn.
    NotSpam,
    /// The report's "Ban" button: ban, clean up, learn.
    ConfirmBan,
}

impl AdminAction {
    /// The rights `self` needs when `warn_limit` warnings mean a ban
    /// (`0` = warnings never ban).
    pub fn privileges(self, warn_limit: u32) -> &'static [Privilege] {
        const BAN_AND_DELETE: &[Privilege] =
            &[Privilege::RestrictMembers, Privilege::DeleteMessages];
        match self {
            AdminAction::Warn if warn_limit == 0 => &[Privilege::DeleteMessages],
            AdminAction::NotSpam => &[Privilege::RestrictMembers],
            AdminAction::Spam | AdminAction::Ban | AdminAction::Warn | AdminAction::ConfirmBan => {
                BAN_AND_DELETE
            }
        }
    }
}

/// Who asks.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Caller {
    /// `ANTISPAM_OWNER_ID`, the operator of this bot.
    Owner,
    /// An admin posting as the group: the person and their rights are unknown.
    AnonymousAdmin,
    /// A member with these admin rights, or `None` if not an admin.
    Member(Option<AdminRights>),
}

/// The answer to a request.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Access {
    Allowed,
    /// Not an admin: the request is an ordinary message.
    NotAdmin,
    /// An admin without a right the action needs (the first one missing).
    MissingRight(Privilege),
    /// An anonymous admin, whose rights cannot be checked.
    Anonymous,
}

/// Who wrote the message an admin command replies to.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Target {
    /// `ANTISPAM_OWNER_ID`.
    Owner,
    /// An admin of the chat, the creator included.
    Admin,
    /// The group itself: an anonymous admin or the linked channel.
    OwnPost,
    /// A member who is not an admin.
    Member,
    /// A channel posting in the group that is not its linked channel.
    Channel,
}

/// Whether admin commands may act on a message from `target`.
pub fn may_target(target: Target) -> bool {
    match target {
        Target::Member | Target::Channel => true,
        Target::Owner | Target::Admin | Target::OwnPost => false,
    }
}

/// Whether `caller` may do something that needs every right in `needed`.
pub fn check(caller: Caller, needed: &[Privilege]) -> Access {
    match caller {
        Caller::Owner => Access::Allowed,
        Caller::AnonymousAdmin => Access::Anonymous,
        Caller::Member(None) => Access::NotAdmin,
        Caller::Member(Some(rights)) => match needed.iter().find(|p| !rights.allow(**p)) {
            None => Access::Allowed,
            Some(missing) => Access::MissingRight(*missing),
        },
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const DELETE_ONLY: AdminRights = AdminRights {
        delete_messages: true,
        restrict_members: false,
    };
    const RESTRICT_ONLY: AdminRights = AdminRights {
        delete_messages: false,
        restrict_members: true,
    };
    const TITLE_ONLY: AdminRights = AdminRights {
        delete_messages: false,
        restrict_members: false,
    };

    #[test]
    fn banning_actions_need_the_ban_right() {
        // Arrange
        let banning = [
            AdminAction::Spam,
            AdminAction::Ban,
            AdminAction::NotSpam,
            AdminAction::ConfirmBan,
        ];

        for action in banning {
            // Act
            let needed = action.privileges(3);

            // Assert
            assert!(needed.contains(&Privilege::RestrictMembers), "{action:?}");
            assert!(action.privileges(0).contains(&Privilege::RestrictMembers));
        }
    }

    #[test]
    fn actions_that_delete_need_the_delete_right_too() {
        // Arrange
        let deleting = [
            AdminAction::Spam,
            AdminAction::Ban,
            AdminAction::Warn,
            AdminAction::ConfirmBan,
        ];
        let ban_only = Caller::Member(Some(RESTRICT_ONLY));

        for action in deleting {
            // Act
            let access = check(ban_only, action.privileges(3));

            // Assert
            assert_eq!(
                access,
                Access::MissingRight(Privilege::DeleteMessages),
                "{action:?}"
            );
        }
        assert_eq!(
            check(ban_only, AdminAction::NotSpam.privileges(3)),
            Access::Allowed
        );
    }

    #[test]
    fn warn_needs_the_ban_right_only_when_warnings_can_ban() {
        // Arrange
        let can_ban = 3;
        let never_bans = 0;

        // Act
        let with_limit = AdminAction::Warn.privileges(can_ban);
        let without_limit = AdminAction::Warn.privileges(never_bans);

        // Assert
        assert_eq!(
            with_limit,
            [Privilege::RestrictMembers, Privilege::DeleteMessages]
        );
        assert_eq!(without_limit, [Privilege::DeleteMessages]);
    }

    #[test]
    fn admin_with_the_right_is_allowed() {
        // Arrange
        let caller = Caller::Member(Some(RESTRICT_ONLY));

        // Act
        let access = check(caller, &[Privilege::RestrictMembers]);

        // Assert
        assert_eq!(access, Access::Allowed);
    }

    #[test]
    fn admin_without_the_right_is_refused() {
        // Arrange
        let delete_only = Caller::Member(Some(DELETE_ONLY));
        let title_only = Caller::Member(Some(TITLE_ONLY));

        // Act
        let ban_by_deleter = check(delete_only, &[Privilege::RestrictMembers]);
        let warn_by_title = check(title_only, &[Privilege::DeleteMessages]);

        // Assert
        assert_eq!(
            ban_by_deleter,
            Access::MissingRight(Privilege::RestrictMembers)
        );
        assert_eq!(
            warn_by_title,
            Access::MissingRight(Privilege::DeleteMessages)
        );
    }

    #[test]
    fn creator_and_owner_may_do_anything() {
        // Arrange
        let creator = Caller::Member(Some(AdminRights::CREATOR));

        for needed in [
            &[Privilege::DeleteMessages][..],
            &[Privilege::RestrictMembers, Privilege::DeleteMessages],
        ] {
            // Act
            let by_creator = check(creator, needed);
            let by_owner = check(Caller::Owner, needed);

            // Assert
            assert_eq!(by_creator, Access::Allowed);
            assert_eq!(by_owner, Access::Allowed);
        }
    }

    #[test]
    fn anonymous_admins_and_members_are_refused() {
        // Arrange
        let needed = &[Privilege::DeleteMessages];

        // Act
        let anonymous = check(Caller::AnonymousAdmin, needed);
        let member = check(Caller::Member(None), needed);

        // Assert
        assert_eq!(anonymous, Access::Anonymous);
        assert_eq!(member, Access::NotAdmin);
    }

    #[test]
    fn commands_spare_admins_the_owner_and_the_group() {
        // Arrange
        let spared = [Target::Owner, Target::Admin, Target::OwnPost];
        let punishable = [Target::Member, Target::Channel];

        // Act
        let spared_allowed: Vec<bool> = spared.into_iter().map(may_target).collect();
        let punishable_allowed: Vec<bool> = punishable.into_iter().map(may_target).collect();

        // Assert
        assert_eq!(spared_allowed, [false, false, false]);
        assert_eq!(punishable_allowed, [true, true]);
    }
}
