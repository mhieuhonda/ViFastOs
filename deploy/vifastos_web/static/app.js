/* ViFastOs landing page — signup form, release feed, stats.
   No frameworks, no external requests other than this origin. */

(function () {
  "use strict";

  var MESSAGES = {
    success: function (n) {
      return "Đã đăng ký — bạn là người thứ " + n + ".";
    },
    already_registered: "Email này đã đăng ký rồi — không cần làm gì thêm.",
    invalid_email: "Email không hợp lệ. Kiểm tra lại định dạng giúp mình nhé.",
    invalid_request: "Dữ liệu gửi lên chưa hợp lệ. Thử lại giúp mình nhé.",
    rate_limited: "Bạn đã đăng ký quá nhiều lần trong một giờ. Thử lại sau nhé.",
    network: "Không gửi được. Kiểm tra kết nối rồi thử lại.",
    server: "Máy chủ đang bận. Thử lại sau ít phút."
  };

  var form = document.getElementById("signup-form");
  var emailInput = document.getElementById("email");
  var noteInput = document.getElementById("note");
  var noteCount = document.getElementById("note-count");
  var submitBtn = document.getElementById("submit-btn");
  var statusEl = document.getElementById("form-status");

  function setStatus(message, kind) {
    if (!statusEl) return;
    statusEl.textContent = message || "";
    statusEl.classList.toggle("is-error", kind === "error");
    statusEl.classList.toggle("is-success", kind === "success");
  }

  function setLoading(loading) {
    if (!submitBtn) return;
    submitBtn.disabled = loading;
    submitBtn.textContent = loading ? "Đang gửi…" : "Đăng ký nhận tin";
    form.setAttribute("aria-busy", loading ? "true" : "false");
  }

  if (noteInput && noteCount) {
    noteInput.addEventListener("input", function () {
      noteCount.textContent = String(noteInput.value.length);
    });
  }

  if (form) {
    form.addEventListener("submit", function (event) {
      event.preventDefault();

      var email = (emailInput.value || "").trim();
      if (!email || email.indexOf("@") < 1) {
        setStatus(MESSAGES.invalid_email, "error");
        emailInput.focus();
        return;
      }

      setLoading(true);
      setStatus("Đang đăng ký…", "info");

      var body = {
        email: email,
        note: (noteInput && noteInput.value.trim()) || null,
        source: "landing"
      };

      fetch("/api/signup", {
        method: "POST",
        headers: { "Content-Type": "application/json" },
        body: JSON.stringify(body)
      })
        .then(function (response) {
          return response
            .json()
            .catch(function () {
              return {};
            })
            .then(function (data) {
              return { status: response.status, data: data };
            });
        })
        .then(function (result) {
          var data = result.data || {};
          if (result.status === 200 && data.ok) {
            setStatus(MESSAGES.success(data.position), "success");
            form.reset();
            form.setAttribute("aria-busy", "false");
            if (noteCount) noteCount.textContent = "0";
            submitBtn.disabled = true;
            submitBtn.textContent = "Đã đăng ký";
            refreshStats();
            return;
          }
          setLoading(false);
          if (result.status === 409) {
            setStatus(MESSAGES.already_registered, "error");
          } else if (result.status === 429) {
            setStatus(MESSAGES.rate_limited, "error");
          } else if (result.status === 422) {
            setStatus(MESSAGES[data.error] || MESSAGES.invalid_email, "error");
          } else {
            setStatus(MESSAGES[data.error] || MESSAGES.server, "error");
          }
        })
        .catch(function () {
          setLoading(false);
          setStatus(MESSAGES.network, "error");
        });
    });
  }

  /* ---------------- stats ---------------- */

  function refreshStats() {
    var line = document.getElementById("stats-line");
    var totalEl = document.getElementById("stats-total");
    if (!line || !totalEl) return;
    fetch("/api/stats", { headers: { Accept: "application/json" } })
      .then(function (r) {
        return r.ok ? r.json() : null;
      })
      .then(function (data) {
        if (!data || typeof data.total !== "number") return;
        totalEl.textContent = String(data.total);
        line.hidden = false;
      })
      .catch(function () {
        /* stats are decorative — fail silently */
      });
  }

  /* ---------------- releases ---------------- */

  function el(tag, className, text) {
    var node = document.createElement(tag);
    if (className) node.className = className;
    if (text !== undefined && text !== null) node.textContent = text;
    return node;
  }

  function formatDate(iso) {
    if (!iso) return "";
    var date = new Date(iso);
    if (isNaN(date.getTime())) return "";
    try {
      return new Intl.DateTimeFormat("vi-VN", {
        day: "2-digit",
        month: "2-digit",
        year: "numeric"
      }).format(date);
    } catch (err) {
      return date.toISOString().slice(0, 10);
    }
  }

  function renderRelease(item) {
    var article = el("article", "release");
    var top = el("div", "release-top");

    var title = el("h3", "release-name");
    var link = el("a", null, item.name || item.tag || "Không tên");
    link.href = item.url || "https://github.com/vifast-oss/ViFastOs/releases";
    link.target = "_blank";
    link.rel = "noopener noreferrer";
    title.appendChild(link);
    top.appendChild(title);

    if (item.tag) top.appendChild(el("span", "pill", item.tag));
    if (item.prerelease) top.appendChild(el("span", "pill pill-pre", "prerelease"));

    var dateText = formatDate(item.published_at);
    if (dateText) top.appendChild(el("span", "release-meta", dateText));

    article.appendChild(top);
    if (item.body_excerpt) {
      article.appendChild(el("p", "release-body", item.body_excerpt));
    }
    return article;
  }

  function renderReleases(data) {
    var root = document.getElementById("releases");
    if (!root) return;
    root.innerHTML = "";
    root.setAttribute("aria-busy", "false");

    var releases = (data && data.releases) || [];
    if (releases.length) {
      releases.forEach(function (item) {
        root.appendChild(renderRelease(item));
      });
      return;
    }

    var box = el("div", "releases-empty");
    if (data && data.error === "unavailable") {
      box.appendChild(el("strong", null, "Không tải được danh sách bản phát hành."));
      box.appendChild(
        el("p", null, "GitHub API không phản hồi. Thử tải lại trang, hoặc xem trực tiếp trên GitHub.")
      );
    } else {
      box.appendChild(el("strong", null, "Chưa có bản phát hành"));
      box.appendChild(
        el("p", null, "Đăng ký để nhận thông báo đầu tiên khi có bản phát hành mới.")
      );
      var cta = el("a", "btn btn-secondary", "Đăng ký nhận tin");
      cta.href = "#dang-ky";
      box.appendChild(cta);
    }
    root.appendChild(box);
  }

  function loadReleases() {
    fetch("/api/releases", { headers: { Accept: "application/json" } })
      .then(function (response) {
        return response.json();
      })
      .then(renderReleases)
      .catch(function () {
        renderReleases({ releases: [], error: "unavailable" });
      });
  }

  var started = false;

  function init() {
    if (started) return;
    started = true;
    refreshStats();
    loadReleases();
  }

  if (document.readyState === "loading") {
    document.addEventListener("DOMContentLoaded", init);
  } else {
    init();
  }
})();
