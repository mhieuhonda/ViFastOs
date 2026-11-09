"""ViFastOs early-access site.

A deliberately small FastAPI application: serves the static landing page,
collects early-access signups into SQLite (stdlib ``sqlite3``, no ORM), proxies
the public GitHub release feed with a 10 minute in-memory cache, and exposes an
admin table plus CSV export behind the shared ``admin_portal`` login form
(password from ``ADMIN_PASSWORD``, signed session cookie) with visit
statistics. No third-party analytics and no raw IP addresses are ever stored.
"""

from __future__ import annotations

import csv
import hashlib
import html
import io
import json
import logging
import os
import re
import sqlite3
import threading
import time
import urllib.request
from contextlib import asynccontextmanager
from datetime import datetime, timedelta, timezone
from pathlib import Path
from typing import Any, Dict, List, Optional

from fastapi import FastAPI, HTTPException, Request
from fastapi.exceptions import RequestValidationError
from fastapi.responses import FileResponse, HTMLResponse, JSONResponse, Response
from fastapi.staticfiles import StaticFiles
from pydantic import BaseModel

from admin_portal import Portal

# --- configuration ---------------------------------------------------------- #

APP_NAME = "ViFastOs"
SITE_URL = os.getenv("SITE_URL", "https://os.vifast.org").rstrip("/")
DATA_DIR = Path(os.getenv("DATA_DIR", "/app/data"))
DB_PATH = DATA_DIR / "vifastos.sqlite3"
STATIC_DIR = Path(__file__).resolve().parent / "static"

ADMIN_PASSWORD = os.getenv("ADMIN_PASSWORD", "")
STATS_SALT = os.getenv("STATS_SALT", "vifastos-early-access-salt")
ALLOWED_HOSTS = tuple(
    h.strip().lower() for h in os.getenv("ALLOWED_HOSTS", "").split(",") if h.strip())

GITHUB_REPO = "vifast-oss/ViFastOs"
GITHUB_RELEASES_URL = f"https://api.github.com/repos/{GITHUB_REPO}/releases?per_page=5"
RELEASES_TTL = 600  # seconds
RATE_LIMIT_MAX = 5  # signups per IP hash ...
RATE_LIMIT_WINDOW = 3600  # ... per hour

EMAIL_RE = re.compile(r"^[A-Za-z0-9._%+\-]+@[A-Za-z0-9.\-]+\.[A-Za-z]{2,}$")

CSP = (
    "default-src 'self'; base-uri 'none'; form-action 'self'; "
    "frame-ancestors 'none'; object-src 'none'; img-src 'self' data:; "
    "script-src 'self'; style-src 'self'; font-src 'self'; "
    "connect-src 'self' https://api.github.com; manifest-src 'self'"
)

logging.basicConfig(
    level=os.getenv("LOG_LEVEL", "INFO"),
    format="%(asctime)s %(levelname)s %(name)s %(message)s",
)
log = logging.getLogger("vifastos")

_db_lock = threading.Lock()
_cache_lock = threading.Lock()
_releases_cache: Dict[str, Any] = {"expires": 0.0, "releases": []}

# --- sqlite ----------------------------------------------------------------- #


def db_connect() -> sqlite3.Connection:
    """Open a connection, creating the data directory on first use."""
    DATA_DIR.mkdir(parents=True, exist_ok=True)
    conn = sqlite3.connect(str(DB_PATH), timeout=10.0)
    conn.row_factory = sqlite3.Row
    return conn


def db_init() -> None:
    """Create the schema lazily; safe to call repeatedly."""
    with _db_lock:
        conn = db_connect()
        try:
            conn.execute(
                """
                CREATE TABLE IF NOT EXISTS signups (
                    id INTEGER PRIMARY KEY,
                    email TEXT UNIQUE NOT NULL,
                    note TEXT,
                    source TEXT,
                    created_at TEXT NOT NULL,
                    ip_hash TEXT
                )
                """
            )
            conn.execute(
                "CREATE INDEX IF NOT EXISTS idx_signups_ip_created "
                "ON signups (ip_hash, created_at)"
            )
            conn.commit()
        finally:
            conn.close()
    log.info("sqlite ready at %s", DB_PATH)


# --- utilities -------------------------------------------------------------- #


def utc_now_iso() -> str:
    return datetime.now(timezone.utc).replace(microsecond=0).isoformat()


def utc_ago_iso(seconds: int) -> str:
    stamp = datetime.now(timezone.utc) - timedelta(seconds=seconds)
    return stamp.replace(microsecond=0).isoformat()


def client_ip(request: Request) -> str:
    """Best-effort client address; the app is expected to sit behind a proxy."""
    forwarded = request.headers.get("x-forwarded-for", "")
    if forwarded:
        return forwarded.split(",")[0].strip()
    real_ip = request.headers.get("x-real-ip", "")
    if real_ip:
        return real_ip.strip()
    return request.client.host if request.client else "unknown"


def hash_ip(ip: str) -> str:
    """Salted one-way hash; the raw address is never persisted."""
    return hashlib.sha256(f"{ip}{STATS_SALT}".encode("utf-8")).hexdigest()


def split_host(value: str) -> str:
    """Strip a port from a Host header, keeping IPv6 literals intact."""
    raw = (value or "").strip().lower()
    if not raw:
        return ""
    if raw.startswith("["):
        end = raw.find("]")
        return raw[1:end] if end != -1 else raw
    return raw.rsplit(":", 1)[0] if raw.count(":") == 1 else raw



class SignupIn(BaseModel):
    email: str
    note: Optional[str] = None
    source: Optional[str] = None


# --- application ------------------------------------------------------------ #


@asynccontextmanager
async def lifespan(_app: FastAPI):
    db_init()
    yield


app = FastAPI(
    title="ViFastOs",
    description="Landing page and early-access signups for the ViFastOs microkernel.",
    version="1.0.0",
    docs_url=None,
    redoc_url=None,
    openapi_url=None,
    lifespan=lifespan,
)

app.mount("/static", StaticFiles(directory=str(STATIC_DIR)), name="static")

# Đăng nhập quản trị bằng form + thống kê truy cập (dùng chung admin_portal.py).
portal = Portal(site="os.vifast.org", title="ViFastOs", db_path=DB_PATH,
                accent="#C0392B", log=log)
portal.install(app)


@app.middleware("http")
async def security_middleware(request: Request, call_next):
    raw_host = request.headers.get("host", "")
    if ALLOWED_HOSTS:
        host = split_host(raw_host)
        if host not in ALLOWED_HOSTS and raw_host.strip().lower() not in ALLOWED_HOSTS:
            log.warning("rejected host header: %r", raw_host)
            return JSONResponse(status_code=400, content={"ok": False, "error": "bad_host"})

    response = await call_next(request)
    headers = response.headers
    headers["X-Content-Type-Options"] = "nosniff"
    headers["Referrer-Policy"] = "strict-origin-when-cross-origin"
    headers["Content-Security-Policy"] = CSP
    headers["X-Frame-Options"] = "DENY"
    headers["Permissions-Policy"] = "camera=(), microphone=(), geolocation=()"
    path = request.url.path
    if path.startswith("/api/") or path.startswith("/admin"):
        headers["Cache-Control"] = "no-store"
    portal.track(request, response.status_code)
    return response


@app.exception_handler(RequestValidationError)
async def validation_error_handler(_request: Request, _exc: RequestValidationError):
    return JSONResponse(status_code=422, content={"ok": False, "error": "invalid_request"})


@app.get("/healthz")
def healthz() -> Dict[str, bool]:
    return {"ok": True}


@app.get("/", include_in_schema=False)
def index() -> FileResponse:
    return FileResponse(STATIC_DIR / "index.html", media_type="text/html")


# --- signups ---------------------------------------------------------------- #


@app.post("/api/signup")
def signup(payload: SignupIn, request: Request) -> JSONResponse:
    email = (payload.email or "").strip().lower()
    if len(email) > 254 or not EMAIL_RE.match(email):
        return JSONResponse(status_code=422, content={"ok": False, "error": "invalid_email"})

    note = (payload.note or "").strip()[:200] or None
    source = (payload.source or "").strip()[:64] or None
    ip_digest = hash_ip(client_ip(request))
    created_at = utc_now_iso()
    window_start = utc_ago_iso(RATE_LIMIT_WINDOW)

    conn = db_connect()
    try:
        with _db_lock:
            if conn.execute("SELECT 1 FROM signups WHERE email = ?", (email,)).fetchone():
                return JSONResponse(
                    status_code=409, content={"ok": False, "error": "already_registered"}
                )

            recent = conn.execute(
                "SELECT COUNT(*) AS n FROM signups WHERE ip_hash = ? AND created_at >= ?",
                (ip_digest, window_start),
            ).fetchone()["n"]
            if recent >= RATE_LIMIT_MAX:
                return JSONResponse(
                    status_code=429, content={"ok": False, "error": "rate_limited"}
                )

            try:
                conn.execute(
                    "INSERT INTO signups (email, note, source, created_at, ip_hash) "
                    "VALUES (?, ?, ?, ?, ?)", (email, note, source, created_at, ip_digest))
                conn.commit()
            except sqlite3.IntegrityError:
                # Lost a race against a concurrent insert of the same address.
                conn.rollback()
                return JSONResponse(
                    status_code=409, content={"ok": False, "error": "already_registered"}
                )

            position = conn.execute("SELECT COUNT(*) AS n FROM signups").fetchone()["n"]
    finally:
        conn.close()

    log.info("signup #%s accepted (source=%s)", position, source or "-")
    return JSONResponse({"ok": True, "position": position})


@app.get("/api/stats")
def stats() -> Dict[str, int]:
    conn = db_connect()
    try:
        total = conn.execute("SELECT COUNT(*) AS n FROM signups").fetchone()["n"]
    finally:
        conn.close()
    return {"total": total}


# --- GitHub releases (cached proxy) ----------------------------------------- #


def _plain_markdown(text: str) -> str:
    """Flatten the small Markdown subset used in release notes (no parser)."""
    text = re.sub(r"\[([^\]]+)\]\([^)]+\)", r"\1", text)  # links -> label
    text = re.sub(r"(?m)^\s{0,3}(#{1,6}|[-*+]|>)\s*", "", text)  # headings/lists/quotes
    return re.sub(r"\s+", " ", text.replace("**", "").replace("`", "")).strip()


def _simplify_release(item: Dict[str, Any]) -> Dict[str, Any]:
    tag = (item.get("tag_name") or item.get("name") or "").strip()
    body = _plain_markdown(item.get("body") or "")
    if len(body) > 280:
        body = body[:279].rstrip() + "…"
    return {
        "tag": tag,
        "name": (item.get("name") or tag).strip(),
        "published_at": item.get("published_at") or item.get("created_at"),
        "url": item.get("html_url") or f"https://github.com/{GITHUB_REPO}/releases",
        "prerelease": bool(item.get("prerelease")),
        "body_excerpt": body,
    }


def fetch_releases() -> List[Dict[str, Any]]:
    """Fetch the latest releases from the public GitHub API (sync, 5s timeout)."""
    req = urllib.request.Request(
        GITHUB_RELEASES_URL,
        headers={
            "Accept": "application/vnd.github+json",
            "User-Agent": f"vifastos-site/1.0 (+{SITE_URL})",
            "X-GitHub-Api-Version": "2022-11-28",
        },
    )
    token = os.getenv("GITHUB_TOKEN", "")
    if token:
        req.add_header("Authorization", f"Bearer {token}")
    with urllib.request.urlopen(req, timeout=5) as response:
        payload = json.loads(response.read().decode("utf-8"))
    if not isinstance(payload, list):
        raise ValueError("unexpected GitHub payload")
    return [_simplify_release(item) for item in payload if isinstance(item, dict)]


@app.get("/api/releases")
def api_releases() -> JSONResponse:
    if time.time() < _releases_cache["expires"]:
        return JSONResponse({"releases": _releases_cache["releases"]})
    try:
        releases = fetch_releases()
    except Exception as exc:  # network, HTTP or JSON problems
        log.warning("github releases unavailable: %s", exc)
        return JSONResponse({"releases": [], "error": "unavailable"})

    with _cache_lock:
        _releases_cache["releases"] = releases
        _releases_cache["expires"] = time.time() + RELEASES_TTL
    return JSONResponse({"releases": releases})


# --- admin ------------------------------------------------------------------ #


def _render_admin(rows: List[sqlite3.Row], total: int, limit: int, panel: str = "") -> str:
    body = "".join(
        "<tr>"
        f"<td>{row['id']}</td>"
        f"<td>{html.escape(row['email'])}</td>"
        f"<td>{html.escape(row['note'] or '')}</td>"
        f"<td>{html.escape(row['source'] or '')}</td>"
        f"<td>{html.escape(row['created_at'])}</td>"
        "</tr>"
        for row in rows
    ) or '<tr><td colspan="5" class="admin-empty">Chưa có đăng ký nào.</td></tr>'

    return f"""<!DOCTYPE html>
<html lang="vi">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<meta name="robots" content="noindex, nofollow">
<title>ViFastOs · signups</title>
<link rel="stylesheet" href="/static/styles.css">
<link rel="stylesheet" href="/admin/portal.css">
</head>
<body class="admin-body">
<main class="admin">
  <h1>Đăng ký sớm · ViFastOs</h1>
  <p class="muted"><a href="/">← Về trang chủ</a> · <a href="/logout">Đăng xuất</a></p>
  {panel}
  <p class="muted">Tổng: <strong>{total}</strong> · hiển thị {min(total, limit)} bản ghi mới nhất. IP chỉ được lưu dưới dạng hash, không lưu địa chỉ thô.</p>
  <p><a class="btn btn-secondary" href="/admin/export.csv">Tải CSV</a></p>
  <div class="admin-table-wrap">
    <table>
      <thead>
        <tr><th>#</th><th>Email</th><th>Ghi chú</th><th>Nguồn</th><th>Thời điểm (UTC)</th></tr>
      </thead>
      <tbody>{body}</tbody>
    </table>
  </div>
</main>
</body>
</html>"""


@app.get("/admin", response_class=HTMLResponse)
def admin(request: Request) -> Response:
    if (denied := portal.guard(request)) is not None:
        return denied
    limit = 500
    conn = db_connect()
    try:
        rows = conn.execute(
            "SELECT id, email, note, source, created_at FROM signups "
            "ORDER BY id DESC LIMIT ?",
            (limit,),
        ).fetchall()
        total = conn.execute("SELECT COUNT(*) AS n FROM signups").fetchone()["n"]
    finally:
        conn.close()
    return HTMLResponse(_render_admin(rows, total, limit, portal.panel_html()))


@app.get("/admin/export.csv")
def admin_export(request: Request) -> Response:
    if (denied := portal.guard(request)) is not None:
        return denied
    conn = db_connect()
    try:
        rows = conn.execute(
            "SELECT id, email, note, source, created_at FROM signups ORDER BY id"
        ).fetchall()
    finally:
        conn.close()

    buffer = io.StringIO()
    writer = csv.writer(buffer)
    writer.writerow(["id", "email", "note", "source", "created_at"])
    for row in rows:
        writer.writerow([row["id"], row["email"], row["note"] or "",
                         row["source"] or "", row["created_at"]])
    return Response(
        content=buffer.getvalue(),
        media_type="text/csv",
        headers={
            "Content-Disposition": 'attachment; filename="vifastos-signups.csv"',
            "Cache-Control": "no-store",
        },
    )


if __name__ == "__main__":
    import uvicorn

    uvicorn.run("app:app", host=os.getenv("HOST", "0.0.0.0"),
                port=int(os.getenv("PORT", "8000")))
