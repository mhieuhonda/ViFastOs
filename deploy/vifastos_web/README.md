# ViFastOs — early-access site

Landing page + early-access signup collector for
[ViFastOs](https://github.com/vifast-oss/ViFastOs), a small seL4-style,
capability-based microkernel written in Rust (`no_std`, MIT OR Apache-2.0).

The app is deliberately tiny: one FastAPI file, stdlib `sqlite3`, three static
files. No ORM, no frontend framework, no external fonts, no analytics, no
cookies. It is meant to run as a single Docker container behind a reverse proxy
at **https://os.vifast.org**.

> Status of the kernel itself: **research / demo**. It boots and runs test
> binaries on QEMU for riscv64, aarch64 and x86_64 — it is *not* production
> ready. Nothing on this site claims otherwise.

## Endpoints

| Method | Path                | Auth              | Description |
| ------ | ------------------- | ----------------- | ----------- |
| GET    | `/`                 | —                 | `static/index.html` landing page |
| GET    | `/healthz`          | —                 | `{"ok": true}` (Docker healthcheck) |
| POST   | `/api/signup`       | —                 | Body `{"email","note?","source?"}` → `200 {"ok":true,"position":N}`, `409 already_registered`, `422 invalid_email`, `429 rate_limited` |
| GET    | `/api/stats`        | —                 | `{"total": N}` — public count only |
| GET    | `/api/releases`     | —                 | GitHub releases proxy, 5 newest, cached 10 min; on upstream failure `200 {"releases": [], "error": "unavailable"}` |
| GET    | `/login`            | —                 | Form đăng nhập quản trị (mật khẩu = `ADMIN_PASSWORD`) |
| GET    | `/logout`           | —                 | Xoá cookie phiên |
| GET    | `/admin`            | phiên đăng nhập   | Thống kê truy cập + bảng đăng ký sớm (500 mới nhất) |
| GET    | `/admin/export.csv` | phiên đăng nhập   | Full CSV export |
| GET    | `/admin/stats.csv`  | phiên đăng nhập   | CSV lượt xem 30 ngày |

`/static/*` is served by `StaticFiles`. OpenAPI/docs routes are disabled on
purpose.

## Environment variables

| Variable         | Default            | Meaning |
| ---------------- | ------------------ | ------- |
| `DATA_DIR`       | `/app/data`        | Directory holding `vifastos.sqlite3` (created lazily) |
| `SITE_URL`       | `https://os.vifast.org` | Public URL, used in the GitHub `User-Agent` |
| `ADMIN_PASSWORD` | *(empty)*          | Password for `/admin` and `/admin/export.csv`; when empty, admin returns `503 admin_disabled` |
| `ALLOWED_HOSTS`  | *(empty)*          | Comma-separated allowlist of `Host` values; when set, other hosts get `400 bad_host` |
| `STATS_SALT`     | dev placeholder    | Salt for `sha256(ip + STATS_SALT)`; **set a real value in production** |
| `PORT`           | `8000`             | Port used by `python app.py` (the container CMD pins 8000) |
| `GITHUB_TOKEN`   | *(empty)*          | Optional; raises the GitHub API rate limit for `/api/releases` |

## Local run

```bash
python3 -m venv .venv
.venv/bin/pip install -r requirements.txt
DATA_DIR=./data ALLOWED_HOSTS= .venv/bin/uvicorn app:app --host 127.0.0.1 --port 8000
```

Open http://127.0.0.1:8000/ — the DB is created on first request/startup under
`./data/`.

Quick check:

```bash
curl -s http://127.0.0.1:8000/healthz
curl -s http://127.0.0.1:8000/api/stats
curl -s -X POST http://127.0.0.1:8000/api/signup \
     -H 'Content-Type: application/json' \
     -d '{"email":"test@example.com","note":"hi","source":"curl"}'
```

## Docker

```bash
mkdir -p data && sudo chown -R 10001:10001 data   # bind mount must be writable by the container uid
docker compose up -d --build                       # listens on 8000, exposed to the Docker network only
docker compose exec web python -c "import urllib.request;print(urllib.request.urlopen('http://127.0.0.1:8000/healthz').read())"
```

Or without compose:

```bash
docker build -t vifastos-web .
docker run -d --name vifastos-web \
  -e SITE_URL=https://os.vifast.org \
  -e ADMIN_PASSWORD='choose-something-long' \
  -e ALLOWED_HOSTS=os.vifast.org \
  -e STATS_SALT='random-string' \
  -v "$PWD/data:/app/data" \
  -p 8000:8000 vifastos-web
```

**Reverse proxy:** `docker-compose.yml` intentionally declares no `networks:`
block — the proxy (Coolify or any other) reaches the container over a Docker
network attached at deploy time. Only `expose: 8000` is published; uncomment the
`ports:` block for local testing without a proxy.

The container runs as uid `10001` (non-root). Mounted `./data` must be writable
by that uid, otherwise SQLite cannot create the database. `/app/data` is set via
`DATA_DIR=/app/data` in the image.

## Data model

```sql
CREATE TABLE IF NOT EXISTS signups (
    id         INTEGER PRIMARY KEY,
    email      TEXT UNIQUE NOT NULL,
    note       TEXT,
    source     TEXT,
    created_at TEXT NOT NULL,   -- ISO-8601 UTC, seconds precision
    ip_hash    TEXT             -- sha256(ip + STATS_SALT), never the raw IP
);
CREATE INDEX IF NOT EXISTS idx_signups_ip_created ON signups (ip_hash, created_at);
```

* Email is trimmed and lowercased before the unique check.
* Rate limit: at most **5 signups per IP hash per hour**, evaluated with a
  `COUNT(*)` over `created_at >= now - 3600s`. Duplicate addresses are reported
  as `409` before the rate-limit check.
* `created_at` strings are fixed-width UTC ISO-8601, so lexicographic comparison
  is a valid time comparison.

## Privacy

* Only the email address, an optional note, an optional source tag, the signup
  timestamp and a salted hash of the IP are stored.
* **The raw IP is never written to disk or logs.** The hash is used solely for
  the per-hour rate limit and cannot be reversed without `STATS_SALT`.
* No cookies, no third-party analytics, no tracking pixels, no external fonts or
  CDNs. The only outbound request the browser makes is to this origin.
* `ip_hash` is not exported by `/admin/export.csv`.
* Removal on request: email **vifastoss@gmail.com**.

## Security notes

* Middleware sets `X-Content-Type-Options`, `Referrer-Policy:
  strict-origin-when-cross-origin`, a restrictive `Content-Security-Policy`
  (self + `https://api.github.com` only), `X-Frame-Options: DENY` and
  `Permissions-Policy`.
* CSP does not allow inline styles or scripts — everything lives in
  `static/styles.css` and `static/app.js`.
* Client IPs come from `X-Forwarded-For` / `X-Real-IP` (the app expects a
  reverse proxy) and fall back to the socket address.
* `/api/*` and `/admin*` responses are sent with `Cache-Control: no-store`.

## Files

```
app.py               FastAPI application (all backend logic)
requirements.txt    pinned ranges: fastapi / uvicorn / pydantic
static/index.html   landing page (Vietnamese, short English subtitle)
static/styles.css   cream + red palette, responsive to 360px
static/app.js       signup form states, release rendering, stats
Dockerfile          python:3.12-slim, non-root, healthcheck
docker-compose.yml  minimal; proxy network configured at deploy time
```

## License

MIT OR Apache-2.0 — same dual license as the kernel (see `LICENSE-MIT` and
`LICENSE-APACHE` at the repo root). ViFastOs itself lives at
https://github.com/vifast-oss/ViFastOs.
