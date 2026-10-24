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
`0.2.0` (точная версия), `0.2` (последний патч ветки 0.2, рекомендуем),
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

Docker ещё не стоит? На Debian/Ubuntu: `sudo apt-get install -y docker.io
docker-compose-v2`. На других системах: <https://docs.docker.com/engine/install/>.

`compose.yaml` перезапускает бота после сбоя и перезагрузки и включает защиту:
файловая система только для чтения, без Linux capabilities, не от root, лимит
памяти 128 МБ, ротация логов.

| Задача | Команда |
|---|---|
| Логи | `docker compose logs -f --tail 100 bot` |
| Поменять настройки | отредактируйте `.env`, затем `docker compose up -d` |
| Обновиться (патчи) | `docker compose pull && docker compose up -d` |
| Перейти на новую минорную или мажорную версию | скачайте новый `compose.yaml` (та же команда, что выше), прочитайте [CHANGELOG](../CHANGELOG.md), затем `docker compose up -d` |
| Закрепить точную версию | добавьте `IMAGE_TAG=0.2.0` в `.env` |
| Остановить | `docker compose down` |

## Просто `docker run`

То же одной командой, без Compose:

```bash
docker run -d --name antispam-shield --restart unless-stopped \
  --env-file .env --read-only --cap-drop ALL \
  --security-opt no-new-privileges:true --memory 128m \
  ghcr.io/privatekey7/telegram-antispam-shield:0.2
```

## Контейнерные платформы

Railway, Render, Fly.io, Koyeb, Northflank, кластер Kubernetes — подойдёт всё,
что запускает Docker-образ как постоянный фоновый процесс:

- **Образ:** `ghcr.io/privatekey7/telegram-antispam-shield:0.2`
- **Переменные:** `BOT_TOKEN` и нужные `ANTISPAM_*` из README.
- **Тип сервиса:** worker / background — без публичного порта и HTTP
  health-check, **один** экземпляр, перезапуск при сбое.
- **Ресурсы:** хватит самого маленького размера (≈64 МБ памяти).

Railway умеет собирать и из вашего форка: **New Project → Deploy from GitHub
repo**, затем добавьте `BOT_TOKEN` во **Variables**. Файл `railway.json`
говорит Railway собирать по `Dockerfile`.

Проверьте цены: у многих платформ больше нет бесплатного тарифа для
постоянно работающих сервисов.

## Из исходников (без Docker)

Нужен Rust 1.88 или новее ([rustup](https://rustup.rs)) и около 2 ГБ
свободной памяти и диска для сборки. Модель встроена в бинарник.

```bash
cargo install --locked --tag v0.2.0 \
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

Если имя бота публичное, любой может добавить его в свою группу и тратить
ресурсы вашего сервера. Укажите ID своих групп в `ANTISPAM_ALLOWED_CHATS`:

1. Остановите бота (или сделайте это до первого запуска).
2. Добавьте бота в группу и напишите там любое сообщение.
3. Откройте в браузере `https://api.telegram.org/bot<ТОКЕН>/getUpdates`
   (никому не показывайте эту ссылку: в ней ваш токен).
4. Найдите `"chat":{"id":-100…` — это отрицательное число и есть ID группы.
   `"from":{"id":…}` — ваш ID для `ANTISPAM_OWNER_ID`.
5. Впишите их в `.env` и запустите бота.
