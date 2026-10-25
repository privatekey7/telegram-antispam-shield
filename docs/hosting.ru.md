# Где запустить бота

🌍 [English](hosting.md) · **Русский**

Бот работает везде, где есть исходящий HTTPS до `api.telegram.org`:

- **Не нужны входящие порты, домен и база данных.** Бот работает через long
  polling.
- **Лёгкий:** около 30 МБ памяти и почти ноль процессора.
- **Должен работать постоянно:** бот модерирует, только пока запущен. Нужна
  машина, которая не выключается: сервер, Raspberry Pi, NAS или ПК без сна.
- **Одна копия на токен.** Telegram отдаёт обновления только одному
  получателю. Перед запуском новой копии остановите старую.

Готовые образы для `linux/amd64` и `linux/arm64` публикуются в
`ghcr.io/privatekey7/telegram-antispam-shield` при каждом релизе. Теги:
`0.3.0` (точная версия), `0.3` (последний патч ветки 0.3, рекомендуем),
`sha-<коммит>`. Тега `latest` нет специально.

Сначала создайте бота и получите токен (README, шаг 1).

> 🤖 **Поручите это ИИ-агенту.** Напишите своему агенту:
> *«Установи Telegram-бота из
> https://github.com/privatekey7/telegram-antispam-shield по его инструкции
> llms-install.md»*. Он сделает всё сам, кроме пары шагов в Telegram, которые
> можете сделать только вы. Подробнее: [llms-install.md](../llms-install.md).

## Docker Compose (рекомендуем)

Подходит для любого Linux-сервера, VPS, домашнего сервера, Raspberry Pi
(64-битная ОС) и NAS с Docker, а также для macOS и Windows с Docker Desktop.

```bash
mkdir antispam-shield && cd antispam-shield
base=https://github.com/privatekey7/telegram-antispam-shield/releases/latest/download
curl -fsSLO "$base/compose.yaml"
curl -fsSL "$base/env.example" -o .env
chmod 600 .env
nano .env                 # впишите BOT_TOKEN=... (и другие настройки, если нужно)
docker compose up -d
docker compose logs -f    # дождитесь "bot_started", затем Ctrl+C
```

Сначала бот не модерирует ни один чат. Добавьте его в свою группу и
разрешите её, как описано в разделе
[Как ограничить бота своими чатами](#как-ограничить-бота-своими-чатами).

Docker ещё не стоит? На Debian/Ubuntu: `sudo apt-get install -y docker.io
docker-compose-v2`. На других системах: <https://docs.docker.com/engine/install/>.

`compose.yaml` перезапускает бота после сбоя и перезагрузки и включает защиту:
файловая система только для чтения, без Linux capabilities, не от root, лимиты
памяти 128 МБ и 128 процессов, ротация логов. Образ distroless: в нём нет ни
shell, ни пакетного менеджера.

| Задача | Команда |
|---|---|
| Логи | `docker compose logs -f --tail 100 bot` |
| Поменять настройки | отредактируйте `.env`, затем `docker compose up -d` |
| Обновиться (патчи) | `docker compose pull && docker compose up -d` |
| Перейти на новую минорную или мажорную версию | скачайте новый `compose.yaml` (та же команда, что выше), прочитайте [CHANGELOG](../CHANGELOG.md), затем `docker compose up -d` |
| Закрепить точную версию | добавьте `IMAGE_TAG=0.3.0` в `.env` |
| Остановить | `docker compose down` |

## Просто `docker run`

То же одной командой, без Compose:

```bash
docker run -d --name antispam-shield --restart unless-stopped \
  --env-file .env --read-only --tmpfs /tmp:size=1m --cap-drop ALL \
  -v antispam-data:/data \
  --security-opt no-new-privileges:true --memory 128m --cpus 0.5 \
  --pids-limit 128 --log-opt max-size=10m --log-opt max-file=3 \
  ghcr.io/privatekey7/telegram-antispam-shield:0.3
```

`--tmpfs /tmp` нужен для проверки здоровья (см. [Проверка здоровья](#проверка-здоровья)).
Указывайте один и тот же `-v antispam-data:/data` при каждом запуске, иначе
новый анонимный том начнёт всё с нуля.

## Контейнерные платформы

Railway, Render, Fly.io, Koyeb, Northflank, кластер Kubernetes — подойдёт всё,
что запускает Docker-образ как постоянный фоновый процесс:

- **Образ:** `ghcr.io/privatekey7/telegram-antispam-shield:0.3`
- **Переменные:** `BOT_TOKEN`, `ANTISPAM_ALLOWED_CHATS` и другие нужные
  `ANTISPAM_*` из README.
- **Тип сервиса:** worker / background — без публичного порта и HTTP
  health-check, **один** экземпляр, перезапуск при сбое.
- **Ресурсы:** хватит самого маленького размера (≈64 МБ памяти).
- **Том (рекомендуем):** подключите небольшой постоянный том в `/data`, чтобы
  доверенные участники, выученные образцы, предупреждения и отчёты переживали
  передеплой. Без него бот работает, но после каждого перезапуска всё это
  забывает, а кнопки старых отчётов перестают работать.

Railway умеет собирать и из вашего форка: **New Project → Deploy from GitHub
repo**, затем добавьте `BOT_TOKEN` во **Variables**. Файл `railway.json`
говорит Railway собирать по `Dockerfile`.

Проверьте цены: у многих платформ больше нет бесплатного тарифа для
постоянно работающих сервисов.

## Из исходников (без Docker)

Нужен Rust 1.88 или новее ([rustup](https://rustup.rs)) и около 2 ГБ
свободной памяти и диска для сборки. Модель встроена в бинарник.

```bash
cargo install --locked --tag v0.3.0 \
  --git https://github.com/privatekey7/telegram-antispam-shield
```

Запускайте из папки, где лежит ваш `.env`:

```bash
~/.cargo/bin/telegram-antispam-shield
```

На Linux запускайте бота как сервис systemd. Скопируйте бинарник в
`/usr/local/bin/`, положите настройки в `/etc/antispam-shield.env` (права
`600`) и создайте `/etc/systemd/system/antispam-shield.service`:

```ini
[Unit]
Description=Telegram Anti-Spam Shield
Wants=network-online.target
After=network-online.target

[Service]
ExecStart=/usr/local/bin/telegram-antispam-shield
EnvironmentFile=/etc/antispam-shield.env
Environment=ANTISPAM_DATA_DIR=/var/lib/antispam-shield
StateDirectory=antispam-shield
DynamicUser=yes
Restart=on-failure
RestartSec=5
NoNewPrivileges=yes
ProtectSystem=strict
ProtectHome=yes
PrivateTmp=yes
MemoryMax=128M

[Install]
WantedBy=multi-user.target
```

```bash
sudo systemctl daemon-reload
sudo systemctl enable --now antispam-shield
journalctl -u antispam-shield -f
```

## Автодеплой из своего форка

Меняете код и хотите, чтобы каждый merge сам выкатывался на ваш сервер? См.
[auto-deploy.md](auto-deploy.md) (на английском).

## Как ограничить бота своими чатами

Имя любого бота публично, поэтому кто угодно может добавить вашего бота в
свою группу. Поэтому бот модерирует только группы из
`ANTISPAM_ALLOWED_CHATS`: в остальных он ничего не читает, не шлёт отчётов и
ничему не учится. Пока список не задан, бот не модерирует ни один чат и пишет
об этом при запуске. Если элемент списка не число, бот не запускается.

**Через лог** (бот запущен):

1. Добавьте бота в группу и напишите там любое сообщение.
2. `docker compose logs bot | grep chat_not_allowed` покажет
   `chat_not_allowed … chat=-100…`. Это отрицательное число и есть ID группы.
3. Впишите его в `.env` (`ANTISPAM_ALLOWED_CHATS=-100…`, несколько через
   запятую) и выполните `docker compose up -d`.

**Через Telegram** (бот остановлен или ещё не запускался):

1. Добавьте бота в группу и напишите там любое сообщение.
2. В папке с `.env` запросите ожидающие апдейты. Токен берётся из `.env` и
   передаётся curl через stdin, поэтому не попадает ни в историю браузера,
   ни в историю shell, ни в список процессов:

   ```bash
   printf 'url = "https://api.telegram.org/bot%s/getUpdates"\n' \
     "$(sed -n 's/^BOT_TOKEN=//p' .env)" | curl -fsS -K -
   ```
3. Найдите `"chat":{"id":-100…` — это отрицательное число и есть ID группы.
   `"from":{"id":…}` — ваш ID для `ANTISPAM_OWNER_ID`.
4. Впишите их в `.env` и запустите бота.

`ANTISPAM_ALLOW_ALL_CHATS=true` заставляет бота модерировать любую группу,
куда его добавят, как было до 0.3. Тогда посторонние смогут тратить ваш
сервер, слать вам отчёты (не больше 20 за 10 минут с одного чата, но чатов
сколько угодно) и занимать общий лимит выученных образцов, так что чату,
который появится позже, может достаться лишь несколько.

## Проверка здоровья

В образе есть `HEALTHCHECK`. После каждого удачного запроса к Telegram бот
пишет время в `/tmp/antispam-heartbeat`, а `telegram-antispam-shield
healthcheck` падает, если отметке больше 90 секунд. `docker compose ps`
показывает `healthy` или `unhealthy`, а
`docker inspect -f '{{.State.Health.Status}}' <контейнер>` печатает только
статус. При `read_only: true` каталог `/tmp` должен быть доступен на запись:
compose.yaml монтирует туда маленький tmpfs. Путь меняет
`ANTISPAM_HEARTBEAT_FILE`.

## Что хранится в базе

`antispam.db` в томе данных хранит по каждому чату:

- **members**: ID участников, сколько у них нормальных сообщений и доверены
  ли они. Недоверенные, молчавшие 30 дней, удаляются; доверенные — когда
  доверие истекает (`ANTISPAM_TRUST_TTL_DAYS`).
- **warnings**: ID участников и число `/warn`; удаляются через месяц после
  последнего предупреждения.
- **samples**: тексты, которые админы отметили как спам или не спам, в виде
  хешей слов, не больше 1000 каждого вида на чат.
- **reports**: последние 1000 отчётов о спаме: чат, ID автора и первые 800
  символов текста, для кнопок отчётов.
- **samples_unscoped**: образцы версий до 0.2.1, не используются (см.
  [ADR 0011](adr/0011-per-chat-bounded-samples.md)). Когда они больше не
  нужны, удалите таблицу: `DROP TABLE samples_unscoped;`.

Больше ничего не хранится: ни истории сообщений, ни имён. Перед обновлением
сохраняйте копию файла (в CHANGELOG сказано, когда меняется схема).

## Проверка образа

Образы релизов начиная с 0.2.1 подписаны [cosign](https://docs.sigstore.dev/)
(без ключей, workflow Release) и несут SBOM в формате SPDX и provenance SLSA.
0.2.0 и более ранние не подписаны.

```bash
image=ghcr.io/privatekey7/telegram-antispam-shield:<версия>
cosign verify "$image" \
  --certificate-oidc-issuer https://token.actions.githubusercontent.com \
  --certificate-identity-regexp '^https://github\.com/privatekey7/telegram-antispam-shield/\.github/workflows/release\.yml@refs/tags/v[0-9]+\.[0-9]+\.[0-9]+$'
docker buildx imagetools inspect "$image" --format '{{ json .SBOM }}'
docker buildx imagetools inspect "$image" --format '{{ json .Provenance }}'
```

Workflow Release публикует только теги, чей коммит есть в `main` и прошёл CI.

Чтобы запускать именно проверенный образ, впишите его digest в `.env`:
`IMAGE_TAG=<версия>@sha256:…` (digest из вывода `cosign verify`). Тег можно
переставить на другой образ, digest нельзя.
