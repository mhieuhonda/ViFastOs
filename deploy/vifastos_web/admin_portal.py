"""ViFast admin portal — đăng nhập bằng mật khẩu + thống kê truy cập.

Một file duy nhất, chỉ dùng FastAPI và thư viện chuẩn. Bản giống nhau được đặt
trong cả bốn trang (vifast.org, key.vifast.org, tts.vifast.org, os.vifast.org)
để mỗi site tự chứa, không phụ thuộc dịch vụ ngoài.

* **Đăng nhập**: form HTML `POST /login`, so khớp hằng thời gian với biến môi
  trường ``ADMIN_PASSWORD``. Phiên là cookie ký HMAC-SHA256 (``vf_admin``),
  không cần bảng session; đổi mật khẩu ⇒ mọi phiên cũ mất hiệu lực ngay.
* **Thống kê**: bảng ``hits``, một dòng cho mỗi lượt xem trang (bỏ qua bot,
  tài nguyên tĩnh và chính các trang quản trị). Dashboard có hôm nay / 7 ngày /
  30 ngày, khách duy nhất, biểu đồ 24 giờ và 14 ngày, trang xem nhiều, nguồn
  giới thiệu, thiết bị, trình duyệt, ngôn ngữ, lượt gần nhất và xuất CSV.
* **Không JavaScript**: chạy được với ``script-src 'self'``; CSS phục vụ từ
  ``/admin/portal.css`` nên chạy được cả với ``style-src 'self'`` (không dùng
  style nội tuyến). Biểu đồ vẽ bằng ký tự khối ▁▂▃▄▅▆▇█.
* **Riêng tư**: không lưu IP thô; chỉ lưu hash có muối (muối đổi mỗi ngày) để
  đếm khách duy nhất.

Cách dùng trong một app FastAPI::

    from admin_portal import Portal

    portal = Portal(site="os.vifast.org", title="ViFastOs", db_path=DB_PATH)
    portal.install(app)

    # trong middleware, sau khi có response:
    portal.track(request, response.status_code)

    # trong route /admin:
    if (denied := portal.guard(request)) is not None:
        return denied

    # chèn bảng thống kê vào HTML quản trị sẵn có:
    body = portal.panel_html()

Biến môi trường:

``ADMIN_PASSWORD``     mật khẩu quản trị (bắt buộc để bật portal)
``ADMIN_SESSION_HOURS`` thời hạn phiên, mặc định 12
``ADMIN_TZ_OFFSET``    múi giờ hiển thị, mặc định 7 (giờ Việt Nam)
``STATS_SALT``         muối thêm cho hash khách truy cập (tuỳ chọn)
"""

from __future__ import annotations

import hashlib
import hmac
import html
import os
import re
import sqlite3
import threading
import time
from datetime import date, datetime, timedelta, timezone
from pathlib import Path
from typing import Any, Dict, Iterable, List, Optional, Sequence, Tuple
from urllib.parse import parse_qs, quote, urlparse

from fastapi import FastAPI, Request
from fastapi.responses import HTMLResponse, RedirectResponse, Response

__all__ = ["Portal", "COOKIE_NAME"]

COOKIE_NAME = "vf_admin"
MAX_PATH_LEN = 200
MAX_RETURN_LEN = 200

# Đường dẫn không tính là lượt xem: trang quản trị, tài nguyên, health check.
SKIP_PREFIXES = (
    "/admin",
    "/login",
    "/logout",
    "/api/",
    "/static/",
    "/brand/",
    "/fonts/",
    "/pkg/",
    "/downloads/",
    "/healthz",
    "/favicon",
)
SKIP_SUFFIXES = (
    ".css", ".js", ".mjs", ".map", ".json", ".txt", ".xml", ".webmanifest",
    ".png", ".jpg", ".jpeg", ".gif", ".webp", ".avif", ".svg", ".ico",
    ".woff", ".woff2", ".ttf", ".otf", ".wasm", ".mp3", ".wav", ".ogg",
    ".zip", ".tar", ".gz", ".pdf", ".apk", ".exe", ".msi", ".dmg", ".vfa",
    ".vfts", ".bin", ".onnx",
)
# Bot / công cụ dòng lệnh — không tính vào thống kê người dùng.
BOT_RE = re.compile(
    r"(bot\b|crawl|spider|slurp|scrape|preview|monitor|uptime|pingdom|"
    r"facebookexternalhit|whatsapp|telegram|discord|headless|phantom|"
    r"curl/|wget/|python-requests|python-urllib|aiohttp|httpx|go-http-client|"
    r"node-fetch|axios|okhttp|java/|libwww)",
    re.IGNORECASE,
)

SPARK = "▁▂▃▄▅▆▇█"

# Bốn trang quản trị của ViFast; dùng cho dải chuyển nhanh ở đầu mỗi trang.
SITES: Tuple[Tuple[str, str, str], ...] = (
    ("vifast.org", "site", "https://vifast.org/admin"),
    ("ViFastOs", "os", "https://os.vifast.org/admin"),
    ("ViFastKey", "key", "https://key.vifast.org/admin"),
    ("ViFastTts", "tts", "https://tts.vifast.org/admin"),
)


def _env_float(name: str, default: float) -> float:
    raw = os.getenv(name, "").strip()
    if not raw:
        return default
    try:
        return float(raw)
    except ValueError:
        return default


def _client_ip(request: Request) -> str:
    """Địa chỉ client sau proxy; không bao giờ được lưu thô."""
    forwarded = request.headers.get("x-forwarded-for", "")
    if forwarded:
        return forwarded.split(",")[0].strip()
    real_ip = request.headers.get("x-real-ip", "")
    if real_ip:
        return real_ip.strip()
    return request.client.host if request.client and request.client.host else ""


def _referer_host(value: str, site: str) -> str:
    """Chỉ giữ host của referer; referer nội bộ coi như truy cập trực tiếp."""
    if not value:
        return ""
    try:
        host = (urlparse(value).hostname or "").lower()
    except ValueError:
        return ""
    if not host:
        return ""
    if host == site or host == f"www.{site}" or site.endswith("." + host):
        return ""
    return host[:80]


def _lang_tag(value: str) -> str:
    """'vi-VN,vi;q=0.9' -> 'vi-VN' (chỉ giữ thẻ ngôn ngữ đầu tiên)."""
    primary = (value or "").split(",")[0].split(";")[0].strip()
    return primary[:10]


def _device(ua: str) -> str:
    low = ua.lower()
    if any(token in low for token in ("ipad", "tablet", "playbook", "silk/")):
        return "máy tính bảng"
    if any(token in low for token in ("mobi", "iphone", "android", "phone")):
        return "điện thoại"
    if "windows" in low or "mac os" in low or "linux" in low or "cros" in low:
        return "máy tính"
    return "khác"


def _browser(ua: str) -> str:
    low = ua.lower()
    order = (
        ("edg/", "Edge"),
        ("opr/", "Opera"),
        ("samsungbrowser", "Samsung Internet"),
        ("firefox/", "Firefox"),
        ("fxios", "Firefox iOS"),
        ("crios", "Chrome iOS"),
        ("chrome/", "Chrome"),
        ("safari/", "Safari"),
        ("webkit", "WebKit"),
    )
    for token, name in order:
        if token in low:
            return name
    return "khác"


def _parse_form(body: bytes) -> Dict[str, str]:
    """Giải mã form urlencoded bằng thư viện chuẩn (không cần python-multipart)."""
    try:
        parsed = parse_qs(body.decode("utf-8", "replace"), keep_blank_values=True)
    except Exception:  # pragma: no cover - body hỏng
        return {}
    return {key: values[0] for key, values in parsed.items() if values}


def _fmt(value: int) -> str:
    """1234 -> 1.234 (dấu chấm ngăn cách kiểu Việt Nam)."""
    return f"{int(value):,}".replace(",", ".")


def _bar_width(value: int, top: int, width: int = 18) -> int:
    """Độ dài cột (số ký tự) cho bảng xếp hạng; 0 khi không có dữ liệu."""
    if not top or value <= 0:
        return 0
    return max(1, min(width, int(round(value * width / top))))


def _spark(values: Sequence[float]) -> str:
    """Biểu đồ cột bằng ký tự khối; 0 hiển thị thành dấu chấm mờ."""
    if not values:
        return ""
    top = max(values) or 1
    out = []
    for value in values:
        if value <= 0:
            out.append("·")
            continue
        index = int(round((value / top) * (len(SPARK) - 1)))
        out.append(SPARK[max(0, min(len(SPARK) - 1, index))])
    return "".join(out)


def _column_chart(values: Sequence[int], rows: int = 4) -> str:
    """Biểu đồ cột dọc bằng ký tự khối (không cần CSS, không cần JS)."""
    if not values:
        return ""
    top = max(values) or 1
    levels = [0 if value <= 0 else max(1, int(round(value * rows / top))) for value in values]
    lines = []
    for row in range(rows, 0, -1):
        lines.append("".join("█" if level >= row else "·" for level in levels))
    return "\n".join(lines)


def _esc(value: Any) -> str:
    return html.escape("" if value is None else str(value), quote=True)


class Portal:
    """Đăng nhập quản trị + ghi nhận và hiển thị thống kê truy cập."""

    def __init__(
        self,
        *,
        site: str,
        title: str,
        db_path: str | Path,
        accent: str = "#C0392B",
        password: Optional[str] = None,
        log: Any = None,
        cookie_secure: Optional[bool] = None,
        favicon: str = "",
        admin_key: str = "",
    ) -> None:
        self.site = site
        self.title = title
        self.db_path = Path(db_path)
        self.accent = accent
        self.password = (password if password is not None
                         else os.getenv("ADMIN_PASSWORD", "")).strip()
        self.session_hours = _env_float("ADMIN_SESSION_HOURS", 12.0)
        self.tz = timezone(timedelta(hours=_env_float("ADMIN_TZ_OFFSET", 7.0)))
        self.log = log
        self._cookie_secure = cookie_secure
        self.favicon = favicon
        self.admin_key = admin_key
        self._local = threading.local()
        self._schema_lock = threading.Lock()
        self._schema_ready = False
        self._attempts: Dict[str, List[float]] = {}
        self._attempts_lock = threading.Lock()
        self.login_max_attempts = int(_env_float("ADMIN_LOGIN_MAX", 6))
        self.login_window = _env_float("ADMIN_LOGIN_WINDOW", 600.0)

    # ------------------------------------------------------------------ tiện ích

    @property
    def disabled(self) -> bool:
        return not self.password

    def _log(self, level: str, message: str, *args: Any) -> None:
        if self.log is not None:
            getattr(self.log, level, self.log.info)(message, *args)

    def _now(self) -> datetime:
        return datetime.now(timezone.utc)

    def _local_day(self, moment: Optional[datetime] = None) -> str:
        if moment is None:
            moment = self._now()
        return moment.astimezone(self.tz).date().isoformat()

    def _local_hour(self, moment: Optional[datetime] = None) -> int:
        if moment is None:
            moment = self._now()
        return moment.astimezone(self.tz).hour

    def _display(self, stamp: str) -> str:
        """ISO UTC -> chuỗi giờ địa phương gọn."""
        try:
            moment = datetime.fromisoformat(stamp.replace("Z", "+00:00"))
        except ValueError:
            return stamp
        return moment.astimezone(self.tz).strftime("%d/%m %H:%M")

    # ------------------------------------------------------------------ sqlite

    def _conn(self) -> sqlite3.Connection:
        conn = getattr(self._local, "conn", None)
        if conn is None:
            self.db_path.parent.mkdir(parents=True, exist_ok=True)
            conn = sqlite3.connect(str(self.db_path), timeout=5.0)
            conn.row_factory = sqlite3.Row
            for pragma in ("journal_mode=WAL", "synchronous=NORMAL", "busy_timeout=4000"):
                try:
                    conn.execute(f"PRAGMA {pragma}")
                except sqlite3.Error:
                    pass
            self._local.conn = conn
        if not self._schema_ready:
            with self._schema_lock:
                if not self._schema_ready:
                    self._create_schema(conn)
                    self._schema_ready = True
        return conn

    @staticmethod
    def _create_schema(conn: sqlite3.Connection) -> None:
        conn.executescript(
            """
            CREATE TABLE IF NOT EXISTS hits (
                id INTEGER PRIMARY KEY AUTOINCREMENT,
                site TEXT NOT NULL,
                ts TEXT NOT NULL,
                day TEXT NOT NULL,
                hour INTEGER NOT NULL,
                path TEXT NOT NULL,
                status INTEGER NOT NULL,
                visitor TEXT NOT NULL,
                referer TEXT NOT NULL DEFAULT '',
                device TEXT NOT NULL DEFAULT '',
                browser TEXT NOT NULL DEFAULT '',
                lang TEXT NOT NULL DEFAULT '',
                source TEXT NOT NULL DEFAULT 'server'
            );
            CREATE INDEX IF NOT EXISTS hits_site_day ON hits (site, day);
            CREATE INDEX IF NOT EXISTS hits_site_ts ON hits (site, ts);
            CREATE INDEX IF NOT EXISTS hits_site_visitor ON hits (site, visitor);
            """
        )
        conn.commit()

    def _query(self, sql: str, params: Sequence[Any] = ()) -> List[sqlite3.Row]:
        try:
            return list(self._conn().execute(sql, params).fetchall())
        except sqlite3.Error as exc:  # pragma: no cover - phòng thủ
            self._log("warning", "stats query failed: %s", exc)
            return []

    # ------------------------------------------------------------------ ghi nhận

    def wanted(self, path: str) -> bool:
        if not path:
            return False
        lowered = path.lower()
        if lowered.startswith(SKIP_PREFIXES):
            return False
        if lowered.endswith(SKIP_SUFFIXES):
            return False
        return True

    def record(
        self,
        *,
        path: str,
        status: int = 200,
        ip: str = "",
        ua: str = "",
        referer: str = "",
        lang: str = "",
        source: str = "server",
    ) -> bool:
        """Ghi một lượt xem; trả True nếu đã ghi."""
        if self.disabled or not self.wanted(path):
            return False
        if ua and BOT_RE.search(ua):
            return False
        if status >= 500:
            return False

        now = self._now()
        day = self._local_day(now)
        hour = self._local_hour(now)
        salt = hashlib.sha256(
            f"{self.site}|{day}|{os.getenv('STATS_SALT', '')}".encode("utf-8")
        ).digest()
        visitor = hashlib.sha256(
            salt + ip.encode("utf-8", "replace") + b"|" + ua.encode("utf-8", "replace")
        ).hexdigest()[:20]

        try:
            conn = self._conn()
            conn.execute(
                "INSERT INTO hits (site, ts, day, hour, path, status, visitor, "
                "referer, device, browser, lang, source) "
                "VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
                (
                    self.site,
                    now.isoformat(timespec="seconds").replace("+00:00", "Z"),
                    day,
                    hour,
                    path[:MAX_PATH_LEN],
                    int(status),
                    visitor,
                    _referer_host(referer, self.site),
                    _device(ua),
                    _browser(ua),
                    _lang_tag(lang),
                    source,
                ),
            )
            conn.commit()
            return True
        except sqlite3.Error as exc:  # pragma: no cover - phòng thủ
            self._log("warning", "stats insert failed: %s", exc)
            return False

    def track(self, request: Request, status_code: int = 200, payload: Optional[Dict[str, Any]] = None) -> None:
        """Hook trong middleware; nuốt lỗi để không làm hỏng trang."""
        if self.disabled:
            return
        try:
            payload = payload or {}
            ua = request.headers.get("user-agent", "")
            path = str(payload.get("p") or request.url.path or "/")
            self.record(
                path=path,
                status=status_code,
                ip=_client_ip(request),
                ua=ua,
                referer=str(payload.get("r") or request.headers.get("referer", "")),
                lang=str(payload.get("l") or request.headers.get("accept-language", "")),
                source=str(payload.get("src") or "server"),
            )
        except Exception as exc:  # pragma: no cover - không được chặn request
            self._log("warning", "stats track failed: %s", exc)

    # ------------------------------------------------------------------ phiên

    def _secret(self) -> bytes:
        material = f"{self.site}|{self.password}|{os.getenv('STATS_SALT', '')}"
        return hashlib.sha256(material.encode("utf-8")).digest()

    def _sign(self, stamp: int) -> str:
        return hmac.new(self._secret(), f"{self.site}:{stamp}".encode(), hashlib.sha256).hexdigest()[:40]

    def _cookie_value(self) -> str:
        stamp = int(time.time())
        return f"{stamp}.{self._sign(stamp)}"

    def _secure(self, request: Request) -> bool:
        """Cookie chỉ gửi qua HTTPS; đoán theo header proxy hoặc theo hostname."""
        if self._cookie_secure is not None:
            return self._cookie_secure
        proto = request.headers.get("x-forwarded-proto", "").split(",")[0].strip().lower()
        if proto:
            return proto == "https"
        if request.url.scheme == "https":
            return True
        # Sau proxy có thể mất header; tên miền công khai luôn đi qua HTTPS.
        host = (request.url.hostname or "").lower()
        return host not in ("", "127.0.0.1", "localhost", "::1", "0.0.0.0", "testserver")

    def auth_ok(self, request: Request) -> bool:
        if self.disabled:
            return False
        raw = request.cookies.get(COOKIE_NAME, "")
        stamp_raw, _, signature = raw.partition(".")
        if not stamp_raw.isdigit() or not signature:
            return False
        stamp = int(stamp_raw)
        if not hmac.compare_digest(signature, self._sign(stamp)):
            return False
        age = time.time() - stamp
        return -60 <= age <= self.session_hours * 3600

    def _rate_limited(self, key: str) -> bool:
        now = time.time()
        with self._attempts_lock:
            bucket = [t for t in self._attempts.get(key, []) if now - t < self.login_window]
            if len(bucket) >= self.login_max_attempts:
                self._attempts[key] = bucket
                return True
            bucket.append(now)
            self._attempts[key] = bucket
            if len(self._attempts) > 2048:  # dọn bộ nhớ khi bị quét
                self._attempts = {k: v for k, v in self._attempts.items() if v and now - v[-1] < self.login_window}
            return False

    def _login_ok(self, password: str) -> bool:
        given = (password or "").encode("utf-8")
        expected = self.password.encode("utf-8")
        return hmac.compare_digest(hashlib.sha256(given).digest(), hashlib.sha256(expected).digest())

    # ------------------------------------------------------------------ phản hồi

    @staticmethod
    def _base_headers(status: int) -> Dict[str, str]:
        return {
            "Cache-Control": "no-store",
            "X-Content-Type-Options": "nosniff",
            "X-Frame-Options": "DENY",
            "Referrer-Policy": "no-referrer",
            "Content-Security-Policy": (
                "default-src 'none'; style-src 'self'; img-src 'self' data:; "
                "form-action 'self'; base-uri 'none'; frame-ancestors 'none'"
            ),
        }

    def _html(self, body: str, status: int = 200) -> HTMLResponse:
        return HTMLResponse(body, status_code=status, headers=self._base_headers(status))

    def _redirect(self, url: str, status: int = 303) -> RedirectResponse:
        return RedirectResponse(url, status_code=status,
                                headers={"Cache-Control": "no-store"})

    def guard(self, request: Request) -> Optional[Response]:
        """None nếu đã đăng nhập; ngược lại là response chuyển tới /login."""
        if self.disabled:
            return self._html(self._page_body(
                "<h1>Admin chưa bật</h1>"
                "<p class=\"vf-muted\">Chưa đặt biến môi trường <code>ADMIN_PASSWORD</code> "
                "cho dịch vụ này.</p>"), status=503)
        if self.auth_ok(request):
            return None
        target = request.url.path
        if request.url.query:
            target = f"{target}?{request.url.query}"
        return self._redirect(f"/login?next={quote(target[:MAX_RETURN_LEN], safe='/')}")

    def site_nav(self) -> str:
        """Dải chuyển nhanh giữa bốn trang quản trị; trang hiện tại được đánh dấu."""
        items = []
        for label, key, url in SITES:
            if key == self.admin_key:
                items.append(
                    f'<span class="vf-nav-item is-current" aria-current="page">{_esc(label)}</span>'
                )
            else:
                items.append(f'<a class="vf-nav-item" href="{_esc(url)}">{_esc(label)}</a>')
        return '<nav class="vf-site-nav" aria-label="Chuyển trang quản trị">' + "".join(items) + "</nav>"

    def shell(self, content: str, *, page_title: str = "") -> str:
        """Khung trang quản trị dùng chung cho cả bốn site."""
        title = page_title or f"{self.title} · Admin"
        mark = (
            f'<img class="vf-shell-mark" src="{_esc(self.favicon)}" alt="" width="26" height="26">'
            if self.favicon
            else ""
        )
        return (
            '<!DOCTYPE html><html lang="vi"><head><meta charset="utf-8">'
            '<meta name="viewport" content="width=device-width, initial-scale=1">'
            '<meta name="robots" content="noindex, nofollow">'
            f"<title>{_esc(title)}</title>"
            + (f'<link rel="icon" href="{_esc(self.favicon)}">' if self.favicon else "")
            + '<link rel="stylesheet" href="/admin/portal.css">'
            "</head><body class=\"vf-admin-body\">"
            '<header class="vf-shell-head">'
            f'<a class="vf-shell-brand" href="/">{mark}<span>{_esc(self.title)} · Admin</span></a>'
            '<div class="vf-shell-tools">'
            + self.site_nav()
            + '<a class="vf-shell-out" href="/logout">Đăng xuất</a>'
            "</div></header>"
            f'<main class="vf-admin">{content}</main>'
            '<p class="vf-shell-foot">ViFast · mã nguồn MIT / Apache-2.0 · '
            '<a href="/">← Về trang chủ</a></p>'
            "</body></html>"
        )

    def topbar(self, home: str = "/", label: str = "← Trang chủ") -> str:
        """Dải điều hướng đầu trang quản trị: về trang chủ + đăng xuất."""
        return (
            '<nav class="vf-topbar" aria-label="Điều hướng Admin">'
            f'<a class="vf-topbar-home" href="{_esc(home)}">{_esc(label)}</a>'
            '<a class="vf-topbar-out" href="/logout">Đăng xuất</a>'
            "</nav>"
        )

    def _page_body(self, inner: str, *, login: bool = False) -> str:
        return (
            "<!DOCTYPE html><html lang=\"vi\"><head><meta charset=\"utf-8\">"
            "<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">"
            "<meta name=\"robots\" content=\"noindex, nofollow\">"
            f"<title>{_esc(self.title)} · Admin</title>"
            + (f"<link rel=\"icon\" href=\"{_esc(self.favicon)}\">" if self.favicon else "")
            + "<link rel=\"stylesheet\" href=\"/admin/portal.css\">"
            "</head><body class=\"vf-admin-body\">"
            f"<main class=\"vf-admin\">{inner}</main></body></html>"
        )

    # ------------------------------------------------------------------ trang đăng nhập

    def _login_page(self, *, next_url: str, error: str = "") -> str:
        notice = f"<p class=\"vf-error\">{_esc(error)}</p>" if error else ""
        return self._page_body(
            "<section class=\"vf-login-card\">"
            f"<p class=\"vf-eyebrow\">{_esc(self.title)}</p>"
            "<h1>Đăng nhập Admin</h1>"
            "<p class=\"vf-muted\">Nhập mật khẩu Admin để xem thống kê truy cập.</p>"
            f"{notice}"
            "<form method=\"post\" action=\"/login\" class=\"vf-form\">"
            f"<input type=\"hidden\" name=\"next\" value=\"{_esc(next_url)}\">"
            "<label for=\"vf-password\">Mật khẩu</label>"
            "<input id=\"vf-password\" name=\"password\" type=\"password\" "
            "autocomplete=\"current-password\" required autofocus>"
            "<button type=\"submit\">Đăng nhập</button>"
            "</form>"
            "<p class=\"vf-foot\"><a href=\"/\">← Về trang chủ</a></p>"
            "</section>"
        )

    @staticmethod
    def _safe_next(raw: str) -> str:
        value = (raw or "").strip()
        if not value.startswith("/") or value.startswith("//") or "\\" in value:
            return "/admin"
        return value[:MAX_RETURN_LEN]

    # ------------------------------------------------------------------ thống kê

    def snapshot(self, *, days: int = 14) -> Dict[str, Any]:
        today = date.fromisoformat(self._local_day())
        day_7 = (today - timedelta(days=6)).isoformat()
        day_30 = (today - timedelta(days=29)).isoformat()
        day_n = (today - timedelta(days=days - 1)).isoformat()

        def counts(day_from: str) -> Tuple[int, int]:
            row = self._query(
                "SELECT COUNT(*) AS views, COUNT(DISTINCT visitor) AS people "
                "FROM hits WHERE site = ? AND day >= ?",
                (self.site, day_from),
            )
            if not row:
                return 0, 0
            return int(row[0]["views"] or 0), int(row[0]["people"] or 0)

        views_today, people_today = counts(today.isoformat())
        views_7, people_7 = counts(day_7)
        views_30, people_30 = counts(day_30)
        total_row = self._query("SELECT COUNT(*) AS n FROM hits WHERE site = ?", (self.site,))
        views_total = int(total_row[0]["n"] or 0) if total_row else 0

        hourly = [0] * 24
        for row in self._query(
            "SELECT hour, COUNT(*) AS n FROM hits WHERE site = ? AND day = ? GROUP BY hour",
            (self.site, today.isoformat()),
        ):
            try:
                hourly[int(row["hour"]) % 24] = int(row["n"])
            except (TypeError, ValueError):
                continue

        daily_rows = self._query(
            "SELECT day, COUNT(*) AS n FROM hits WHERE site = ? AND day >= ? "
            "GROUP BY day ORDER BY day",
            (self.site, day_n),
        )
        daily_map = {str(row["day"]): int(row["n"]) for row in daily_rows}
        daily: List[Tuple[str, int]] = []
        for offset in range(days - 1, -1, -1):
            key = (today - timedelta(days=offset)).isoformat()
            daily.append((key, daily_map.get(key, 0)))

        top_paths = [
            (str(row["path"]), int(row["n"]))
            for row in self._query(
                "SELECT path, COUNT(*) AS n FROM hits WHERE site = ? AND day >= ? "
                "GROUP BY path ORDER BY n DESC LIMIT 8",
                (self.site, day_7),
            )
        ]
        referers = [
            (str(row["referer"] or ""), int(row["n"]))
            for row in self._query(
                "SELECT referer, COUNT(*) AS n FROM hits WHERE site = ? AND day >= ? "
                "GROUP BY referer ORDER BY n DESC LIMIT 6",
                (self.site, day_7),
            )
        ]
        devices = [
            (str(row["device"] or "khác"), int(row["n"]))
            for row in self._query(
                "SELECT device, COUNT(*) AS n FROM hits WHERE site = ? AND day >= ? "
                "GROUP BY device ORDER BY n DESC LIMIT 5",
                (self.site, day_7),
            )
        ]
        browsers = [
            (str(row["browser"] or "khác"), int(row["n"]))
            for row in self._query(
                "SELECT browser, COUNT(*) AS n FROM hits WHERE site = ? AND day >= ? "
                "GROUP BY browser ORDER BY n DESC LIMIT 5",
                (self.site, day_7),
            )
        ]
        langs = [
            (str(row["lang"] or "không rõ"), int(row["n"]))
            for row in self._query(
                "SELECT lang, COUNT(*) AS n FROM hits WHERE site = ? AND day >= ? "
                "GROUP BY lang ORDER BY n DESC LIMIT 5",
                (self.site, day_7),
            )
        ]
        recent = [
            {
                "ts": str(row["ts"]),
                "path": str(row["path"]),
                "status": int(row["status"]),
                "device": str(row["device"]),
                "referer": str(row["referer"]),
            }
            for row in self._query(
                "SELECT ts, path, status, device, referer FROM hits "
                "WHERE site = ? ORDER BY id DESC LIMIT 12",
                (self.site,),
            )
        ]
        last_row = self._query("SELECT MAX(ts) AS ts FROM hits WHERE site = ?", (self.site,))
        last_seen = str(last_row[0]["ts"]) if last_row and last_row[0]["ts"] else ""

        return {
            "views_today": views_today,
            "people_today": people_today,
            "views_7": views_7,
            "people_7": people_7,
            "views_30": views_30,
            "people_30": people_30,
            "views_total": views_total,
            "hourly": hourly,
            "daily": daily,
            "top_paths": top_paths,
            "referers": referers,
            "devices": devices,
            "browsers": browsers,
            "langs": langs,
            "recent": recent,
            "last_seen": last_seen,
            "today": today.isoformat(),
        }

    def _bars(self, rows: Iterable[Tuple[str, int]], top: int) -> str:
        cells = []
        for label, value in rows:
            share = _bar_width(value, top)
            # Bề rộng cột vẽ bằng số ô ký tự, không dùng style nội tuyến (CSP).
            cells.append(
                "<tr>"
                f"<th scope=\"row\" title=\"{_esc(label)}\">{_esc(label)}</th>"
                f"<td class=\"vf-bar\">{SPARK[-1] * share}</td>"
                f"<td class=\"vf-num\">{_fmt(value)}</td>"
                "</tr>"
            )
        return "".join(cells)

    def panel_html(self, *, days: int = 14) -> str:
        """Khối HTML thống kê, chèn được vào bất kỳ trang quản trị nào."""
        data = self.snapshot(days=days)
        hourly = data["hourly"]
        daily_values = [n for _, n in data["daily"]]
        daily_labels = [d for d, _ in data["daily"]]
        top_paths = data["top_paths"]
        max_path = max((n for _, n in top_paths), default=0)
        max_ref = max((n for _, n in data["referers"]), default=0)

        def card(label: str, views: int, people: int) -> str:
            return (
                "<div class=\"vf-card\">"
                f"<span class=\"vf-card-label\">{_esc(label)}</span>"
                f"<span class=\"vf-card-value\">{_fmt(views)}</span>"
                f"<span class=\"vf-card-sub\">{_fmt(people)} khách</span>"
                "</div>"
            )

        referer_rows = "".join(
            "<tr>"
            f"<th scope=\"row\">{_esc(host or 'truy cập trực tiếp')}</th>"
            f"<td class=\"vf-bar\">{SPARK[-1] * _bar_width(n, max_ref)}</td>"
            f"<td class=\"vf-num\">{_fmt(n)}</td>"
            "</tr>"
            for host, n in data["referers"]
        ) or "<tr><td colspan=\"3\" class=\"vf-muted\">Chưa có dữ liệu.</td></tr>"

        path_rows = self._bars(top_paths, max_path) or (
            "<tr><td colspan=\"3\" class=\"vf-muted\">Chưa có dữ liệu.</td></tr>"
        )

        def mini_table(title: str, rows: Sequence[Tuple[str, int]]) -> str:
            body = "".join(
                f"<tr><th scope=\"row\">{_esc(name)}</th><td class=\"vf-num\">{_fmt(n)}</td></tr>"
                for name, n in rows
            ) or "<tr><td class=\"vf-muted\">Chưa có dữ liệu.</td></tr>"
            return (
                f"<div class=\"vf-mini\"><h3>{_esc(title)}</h3>"
                f"<table class=\"vf-table vf-compact\"><tbody>{body}</tbody></table></div>"
            )

        recent_rows = "".join(
            "<tr>"
            f"<td class=\"vf-mono\">{_esc(self._display(item['ts']))}</td>"
            f"<td class=\"vf-mono\">{_esc(item['path'])}</td>"
            f"<td class=\"vf-num\">{item['status']}</td>"
            f"<td>{_esc(item['device'])}</td>"
            f"<td>{_esc(item['referer'] or '—')}</td>"
            "</tr>"
            for item in data["recent"]
        ) or "<tr><td colspan=\"5\" class=\"vf-muted\">Chưa có lượt truy cập nào.</td></tr>"

        updated = self._display(data["last_seen"]) if data["last_seen"] else "—"
        return f"""
<section class="vf-panel" id="thong-ke">
  <header class="vf-panel-head">
    <div>
      <h2>Thống kê truy cập</h2>
      <p class="vf-muted">Chỉ đếm lượt xem trang của người dùng (bỏ qua bot, tài nguyên tĩnh).
      Giờ Việt Nam (UTC+7). Cập nhật: <strong>{_esc(updated)}</strong>.</p>
    </div>
    <a class="vf-link" href="/admin/stats.csv">Tải CSV 30 ngày</a>
  </header>

  <div class="vf-cards">
    {card("Hôm nay", data["views_today"], data["people_today"])}
    {card("7 ngày", data["views_7"], data["people_7"])}
    {card("30 ngày", data["views_30"], data["people_30"])}
    <div class="vf-card"><span class="vf-card-label">Tổng đã ghi</span>
      <span class="vf-card-value">{_fmt(data["views_total"])}</span>
      <span class="vf-card-sub">từ khi bật portal</span></div>
  </div>

  <div class="vf-chart">
    <h3>24 giờ hôm nay <span class="vf-muted">(mỗi cột = 1 giờ, 0h → 23h)</span></h3>
    <pre class="vf-bars" aria-label="Lượt xem theo giờ">{_esc(_column_chart(hourly, 4))}</pre>
    <div class="vf-axis"><span>00h</span><span>06h</span><span>12h</span><span>18h</span><span>23h</span></div>
  </div>

  <div class="vf-chart">
    <h3>{days} ngày gần nhất</h3>
    <pre class="vf-bars" aria-label="Lượt xem theo ngày">{_esc(_column_chart(daily_values, 4))}</pre>
    <div class="vf-axis"><span>{_esc(daily_labels[0][5:])}</span><span>{_esc(daily_labels[-1][5:])}</span></div>
  </div>

  <div class="vf-grid">
    <div class="vf-col">
      <h3>Trang xem nhiều (7 ngày)</h3>
      <table class="vf-table vf-rank"><tbody>{path_rows}</tbody></table>
    </div>
    <div class="vf-col">
      <h3>Nguồn giới thiệu (7 ngày)</h3>
      <table class="vf-table vf-rank"><tbody>{referer_rows}</tbody></table>
    </div>
  </div>

  <div class="vf-grid vf-grid-3">
    {mini_table("Thiết bị", data["devices"])}
    {mini_table("Trình duyệt", data["browsers"])}
    {mini_table("Ngôn ngữ", data["langs"])}
  </div>

  <h3>Lượt xem gần nhất</h3>
  <div class="vf-scroll">
    <table class="vf-table">
      <thead><tr><th>Thời điểm</th><th>Đường dẫn</th><th>Mã</th><th>Thiết bị</th><th>Nguồn</th></tr></thead>
      <tbody>{recent_rows}</tbody>
    </table>
  </div>
</section>
"""

    def csv_rows(self, days: int = 30) -> str:
        today = date.fromisoformat(self._local_day())
        since = (today - timedelta(days=days - 1)).isoformat()
        rows = self._query(
            "SELECT day, path, COUNT(*) AS views, COUNT(DISTINCT visitor) AS people "
            "FROM hits WHERE site = ? AND day >= ? GROUP BY day, path ORDER BY day, views DESC",
            (self.site, since),
        )
        lines = ["ngay,duong_dan,luot_xem,khach"]
        for row in rows:
            lines.append(
                f"{row['day']},{self._csv(str(row['path']))},{row['views']},{row['people']}"
            )
        return "\n".join(lines) + "\n"

    @staticmethod
    def _csv(value: str) -> str:
        text = value.replace('"', "'")
        return f'"{text}"' if "," in text else text

    # ------------------------------------------------------------------ CSS

    def css(self) -> str:
        accent = re.sub(r"[^#0-9A-Fa-f]", "", self.accent) or "#C0392B"
        return f"""
:root {{
  --vf-accent: {accent};
  --vf-accent-soft: #FBEDE9;
  --vf-bg: #FFF8EC;
  --vf-card: #FFFCF7;
  --vf-ink: #2E2119;
  --vf-muted: #7A6558;
  --vf-line: #EADBCC;
}}
* {{ box-sizing: border-box; }}
body.vf-admin-body {{
  margin: 0; min-height: 100vh; background: var(--vf-bg); color: var(--vf-ink);
  font-family: -apple-system, BlinkMacSystemFont, "Segoe UI", Roboto, "Helvetica Neue",
    Arial, "Liberation Sans", "DejaVu Sans", sans-serif;
  font-size: 15px; line-height: 1.55; -webkit-font-smoothing: antialiased;
}}
.vf-admin {{ max-width: 1040px; margin: 0 auto; padding: 28px 18px 64px; }}
.vf-admin a {{ color: var(--vf-accent); }}
.vf-muted {{ color: var(--vf-muted); }}
.vf-mono, code, pre {{ font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace; }}
h1, h2, h3 {{ line-height: 1.25; margin: 0 0 8px; }}
h1 {{ font-size: 1.6rem; }}
h2 {{ font-size: 1.25rem; }}
h3 {{ font-size: .95rem; margin-top: 22px; color: var(--vf-muted); text-transform: uppercase; letter-spacing: .04em; }}

/* --- đăng nhập --- */
.vf-login-card {{
  max-width: 420px; margin: 9vh auto 0; background: var(--vf-card);
  border: 1px solid var(--vf-line); border-radius: 18px; padding: 28px;
  box-shadow: 0 18px 40px -28px rgba(60, 30, 15, .45);
}}
.vf-eyebrow {{
  margin: 0 0 6px; font-size: .72rem; font-weight: 700; letter-spacing: .12em;
  text-transform: uppercase; color: var(--vf-accent);
}}
.vf-form {{ display: grid; gap: 10px; margin: 18px 0 6px; }}
.vf-form label {{ font-weight: 600; font-size: .9rem; }}
.vf-form input[type=password] {{
  width: 100%; padding: 11px 12px; border-radius: 10px; font: inherit;
  border: 1px solid var(--vf-line); background: #fff; color: var(--vf-ink);
}}
.vf-form input[type=password]:focus-visible {{ outline: 2px solid var(--vf-accent); outline-offset: 2px; }}
.vf-form button {{
  margin-top: 4px; padding: 11px 16px; border: 0; border-radius: 999px; cursor: pointer;
  font: inherit; font-weight: 700; color: #fff; background: var(--vf-accent);
}}
.vf-form button:hover {{ filter: brightness(1.06); }}
.vf-error {{
  margin: 12px 0 0; padding: 9px 12px; border-radius: 10px; font-size: .9rem;
  background: #FCEBE7; color: #8E1E12; border: 1px solid #F2C9C1;
}}
.vf-foot {{ margin: 18px 0 0; font-size: .88rem; }}

/* --- panel thống kê --- */
.vf-shell-head {{
  display: flex; flex-wrap: wrap; gap: 12px 16px; align-items: center;
  justify-content: space-between; max-width: 1040px; margin: 0 auto; padding: 18px 18px 0;
}}
.vf-shell-brand {{
  display: inline-flex; align-items: center; gap: 9px; text-decoration: none;
  color: var(--vf-ink); font-weight: 800; font-size: 1.05rem;
}}
.vf-shell-mark {{ border-radius: 7px; display: block; }}
.vf-shell-tools {{ display: flex; flex-wrap: wrap; gap: 10px; align-items: center; }}
.vf-site-nav {{ display: flex; flex-wrap: wrap; gap: 4px; }}
.vf-nav-item {{
  padding: 5px 11px; border-radius: 999px; font-size: .8rem; font-weight: 600;
  text-decoration: none; color: var(--vf-muted); border: 1px solid transparent;
}}
a.vf-nav-item:hover {{ color: var(--vf-accent); border-color: var(--vf-line); }}
.vf-nav-item.is-current {{ background: var(--vf-accent-soft); color: var(--vf-accent); }}
.vf-shell-out {{
  padding: 5px 12px; border-radius: 999px; font-size: .8rem; font-weight: 600;
  text-decoration: none; color: var(--vf-muted); border: 1px solid var(--vf-line);
}}
.vf-shell-out:hover {{ color: var(--vf-accent); border-color: var(--vf-accent); }}
.vf-shell-foot {{
  max-width: 1040px; margin: 30px auto 0; padding: 0 18px 44px;
  font-size: .8rem; color: var(--vf-muted);
}}
.vf-h2 {{
  margin: 30px 0 6px; font-size: .95rem; letter-spacing: .05em;
  text-transform: uppercase; color: var(--vf-muted); font-weight: 700;
}}
.vf-note {{
  margin: 24px 0 0; padding: 10px 12px; border-radius: 10px; font-size: .84rem;
  background: #FFF7E6; border: 1px solid #F0D49A; color: #7A4B00;
}}
.vf-tiny {{
  padding: 4px 9px; font-size: .78rem; border-radius: 8px; cursor: pointer;
  border: 1px solid var(--vf-line); background: var(--vf-card); color: var(--vf-ink);
}}
.vf-tiny-ok {{ background: #E5F5EC; border-color: #B8E2C8; color: #166534; }}
.vf-pill {{
  display: inline-block; padding: 2px 8px; border-radius: 999px;
  font-size: .76rem; background: var(--vf-accent-soft); color: var(--vf-accent); white-space: nowrap;
}}
.vf-pill-ok {{ background: #E5F5EC; color: #166534; }}
.vf-pill-new {{ background: #FFF4D6; color: #7A4B00; }}
.vf-msg {{ max-width: 380px; white-space: normal; line-height: 1.45; overflow-wrap: anywhere; }}
.vf-form-inline {{ display: inline; }}
.vf-topbar {{
  display: flex; flex-wrap: wrap; gap: 8px; justify-content: flex-end; align-items: center;
  margin: 0 0 18px;
}}
.vf-topbar a {{
  display: inline-flex; align-items: center; padding: 7px 14px; border-radius: 999px;
  font-size: .85rem; font-weight: 600; text-decoration: none; color: var(--vf-ink);
  background: var(--vf-card); border: 1px solid var(--vf-line);
}}
.vf-topbar a:hover {{ border-color: var(--vf-accent); color: var(--vf-accent); }}
.vf-topbar a.vf-topbar-home {{ background: var(--vf-accent); border-color: var(--vf-accent); color: #fff; }}
.vf-topbar a.vf-topbar-home:hover {{ filter: brightness(1.06); color: #fff; }}
.vf-panel {{ margin: 6px 0 30px; padding-bottom: 8px; border-bottom: 1px solid var(--vf-line); }}
.vf-panel-head {{
  display: flex; flex-wrap: wrap; gap: 10px 18px; align-items: baseline;
  justify-content: space-between; margin-bottom: 14px;
}}
.vf-panel-head p {{ margin: 0; font-size: .88rem; }}
.vf-link {{ font-size: .88rem; font-weight: 600; }}
.vf-cards {{ display: grid; grid-template-columns: repeat(auto-fit, minmax(150px, 1fr)); gap: 12px; }}
.vf-card {{
  background: var(--vf-card); border: 1px solid var(--vf-line); border-radius: 14px;
  padding: 14px 16px; display: grid; gap: 2px;
}}
.vf-card-label {{ font-size: .78rem; text-transform: uppercase; letter-spacing: .06em; color: var(--vf-muted); }}
.vf-card-value {{ font-size: 1.9rem; font-weight: 800; line-height: 1.1; }}
.vf-card-sub {{ font-size: .82rem; color: var(--vf-muted); }}
.vf-chart {{ margin-top: 18px; }}
.vf-spark {{
  margin: 6px 0 2px; font-size: 1.6rem; letter-spacing: .08em; line-height: 1.1;
  color: var(--vf-accent); overflow-x: auto; white-space: nowrap;
}}
.vf-bars {{
  margin: 8px 0 4px; font-family: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
  font-size: 1rem; line-height: 1.08; letter-spacing: .06em; color: var(--vf-accent);
  white-space: pre; overflow-x: auto;
}}
.vf-axis {{ display: flex; justify-content: space-between; font-size: .75rem; color: var(--vf-muted); }}
.vf-grid {{ display: grid; grid-template-columns: repeat(auto-fit, minmax(280px, 1fr)); gap: 22px; }}
.vf-grid-3 {{ grid-template-columns: repeat(auto-fit, minmax(180px, 1fr)); }}
.vf-table {{ width: 100%; border-collapse: collapse; font-size: .9rem; }}
.vf-table th, .vf-table td {{ padding: 7px 8px; border-bottom: 1px solid var(--vf-line); text-align: left; }}
.vf-table thead th {{ font-size: .76rem; text-transform: uppercase; letter-spacing: .05em; color: var(--vf-muted); }}
.vf-table th[scope=row] {{ font-weight: 500; max-width: 260px; overflow: hidden; text-overflow: ellipsis; white-space: nowrap; }}
/* Trung hoà style bảng của trang chủ nhúng (nền, chữ hoa, giãn chữ). */
.vf-panel .vf-table th,
.vf-panel .vf-table td {{
  background: transparent; text-transform: none; letter-spacing: 0; color: var(--vf-ink);
  font-size: .9rem; white-space: normal; vertical-align: middle;
}}
.vf-panel .vf-table thead th {{
  text-transform: uppercase; letter-spacing: .05em; color: var(--vf-muted); font-size: .76rem;
}}
.vf-panel .vf-table th[scope=row] {{ white-space: nowrap; }}
.vf-panel .vf-table td.vf-bar {{ color: var(--vf-accent); font-size: .78rem; letter-spacing: 0; }}
/* Trang nhúng có thể tô nền cho cả <table>; bỏ để panel giữ nền kem. */
.vf-panel .vf-table,
.vf-panel .vf-table thead,
.vf-panel .vf-table tbody,
.vf-panel .vf-table tr {{ background: transparent; border: 0; }}
.vf-panel .vf-table {{ border-radius: 0; }}
.vf-compact th, .vf-compact td {{ padding: 5px 6px; }}
.vf-bar {{ color: var(--vf-accent); letter-spacing: 0; white-space: nowrap; font-size: .8rem; overflow: hidden; }}
.vf-rank {{ table-layout: fixed; }}
.vf-rank th[scope=row] {{ width: 42%; }}
.vf-rank td.vf-bar {{ width: 48%; }}
.vf-rank td.vf-num {{ width: 10%; }}
.vf-num {{ text-align: right; font-variant-numeric: tabular-nums; font-weight: 600; }}
.vf-scroll {{ overflow-x: auto; }}
.vf-mini h3 {{ margin-top: 18px; }}
@media (max-width: 560px) {{
  .vf-card-value {{ font-size: 1.5rem; }}
  .vf-spark {{ font-size: 1.15rem; letter-spacing: .04em; }}
}}
"""

    # ------------------------------------------------------------------ route

    def install(self, app: FastAPI, *, dashboard: Optional[str] = None) -> "Portal":
        """Gắn route đăng nhập, CSS, CSV (và tuỳ chọn dashboard) vào app."""

        portal = self

        @app.get("/login", include_in_schema=False)
        def login_form(request: Request, next: str = "/admin") -> Response:  # noqa: A002
            target = portal._safe_next(next)
            if portal.auth_ok(request):
                return portal._redirect(target)
            if portal.disabled:
                return portal.guard(request)  # trả trang 503
            return portal._html(portal._login_page(next_url=target))

        @app.post("/login", include_in_schema=False)
        async def login_submit(request: Request) -> Response:
            if portal.disabled:
                return portal.guard(request)
            form = _parse_form(await request.body())
            target = portal._safe_next(str(form.get("next") or "/admin"))
            password = str(form.get("password") or "")
            limited = portal._rate_limited(_client_ip(request) or "unknown")
            if not limited and portal._login_ok(password):
                response = portal._redirect(target)
                max_age = int(portal.session_hours * 3600)
                response.set_cookie(
                    COOKIE_NAME,
                    portal._cookie_value(),
                    max_age=max_age,
                    path="/",
                    httponly=True,
                    samesite="lax",
                    secure=portal._secure(request),
                )
                return response
            message = ("Quá nhiều lần thử. Đợi vài phút rồi thử lại."
                       if limited else "Sai mật khẩu. Thử lại.")
            status = 429 if limited else 401
            return portal._html(portal._login_page(next_url=target, error=message), status=status)

        @app.get("/logout", include_in_schema=False)
        def logout() -> Response:
            response = portal._redirect("/")
            response.delete_cookie(COOKIE_NAME, path="/")
            return response

        @app.get("/admin/portal.css", include_in_schema=False)
        def portal_css() -> Response:
            return Response(
                portal.css(),
                media_type="text/css; charset=utf-8",
                headers={"Cache-Control": "no-cache"},
            )

        @app.get("/admin/stats.csv", include_in_schema=False)
        def stats_csv(request: Request) -> Response:
            denied = portal.guard(request)
            if denied is not None:
                return denied
            return Response(
                portal.csv_rows(),
                media_type="text/csv; charset=utf-8",
                headers={
                    "Content-Disposition": 'attachment; filename="vifast-stats.csv"',
                    "Cache-Control": "no-store",
                },
            )

        if dashboard is not None:
            @app.get("/admin", include_in_schema=False)
            def admin_dashboard(request: Request) -> Response:
                denied = portal.guard(request)
                if denied is not None:
                    return denied
                content = (
                    f'<p class="vf-muted">{dashboard or ""}</p>'
                    + portal.panel_html()
                )
                return portal._html(portal.shell(content))

        return self
