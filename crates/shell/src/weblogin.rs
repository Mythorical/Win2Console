//! Controller-driven sign-in pages: a WebView2 child window hosting a service's real login page.
//!
//! An injected script gives the page spatial D-pad navigation (focused element outlined in the accent
//! colour), reports which element is focused, and accepts text from our on-screen keyboard. The
//! navigation handler watches for the OAuth redirect and hands the authorization code back to the app.

use base64::Engine;
use sha2::{Digest, Sha256};
use std::path::PathBuf;

use wry::{WebContext, WebView, WebViewBuilder, dpi};

/// What the page's script reports about the focused element.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct FocusInfo {
    /// A text-like input (our keyboard can type into it).
    pub text: bool,
    pub password: bool,
    pub value: String,
    pub label: String,
}

/// Messages from the page or the navigation handler, delivered on the UI thread.
#[derive(Debug, Clone)]
pub enum WebEvent {
    Focus(FocusInfo),
    /// The OAuth redirect arrived with these query parameters.
    Redirect(Vec<(String, String)>),
    /// A watched page loaded (sign-ins that show their result as page text, e.g. Epic).
    Page {
        url: String,
        body: String,
    },
}

/// A PKCE pair for an OAuth authorization-code flow.
#[derive(Debug, Clone)]
pub struct Pkce {
    pub verifier: String,
    pub challenge: String,
    pub state: String,
}

impl Pkce {
    pub fn new() -> Self {
        let verifier = random_token(48);
        let challenge = b64url(&Sha256::digest(verifier.as_bytes()));
        Self { verifier, challenge, state: random_token(16) }
    }
}

fn random_token(bytes: usize) -> String {
    let mut buf = vec![0u8; bytes];
    getrandom::fill(&mut buf).expect("OS random number generator");
    b64url(&buf)
}

fn b64url(bytes: &[u8]) -> String {
    base64::engine::general_purpose::URL_SAFE_NO_PAD.encode(bytes)
}

/// Percent-encodes a query parameter value.
pub fn encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => out.push(b as char),
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

fn decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        match bytes[i] {
            b'%' if i + 2 < bytes.len() => {
                let hex = std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("");
                match u8::from_str_radix(hex, 16) {
                    Ok(v) => {
                        out.push(v);
                        i += 3;
                        continue;
                    }
                    Err(_) => out.push(b'%'),
                }
            }
            b'+' => out.push(b' '),
            b => out.push(b),
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Query parameters of a URL.
pub fn query_params(url: &str) -> Vec<(String, String)> {
    let query = url.split_once('?').map(|(_, q)| q).unwrap_or("");
    let query = query.split('#').next().unwrap_or("");
    query
        .split('&')
        .filter(|kv| !kv.is_empty())
        .map(|kv| {
            let (k, v) = kv.split_once('=').unwrap_or((kv, ""));
            (decode(k), decode(v))
        })
        .collect()
}

/// Script injected into every page: spatial navigation, focus reporting, text entry.
fn page_script(accent: &str, page_prefix: &str) -> String {
    r#"
(() => {
  if (window.__cs) return;
  const ACCENT = "__ACCENT__";
  const SEL = 'a[href],button,input:not([type=hidden]),select,textarea,[role=button],[role=link],[role=checkbox],[role=option],[tabindex]:not([tabindex="-1"])';
  const textTypes = ['text','email','password','search','tel','url','number',''];
  const cs = window.__cs = {
    cur: null, saved: '',
    items() {
      return [...document.querySelectorAll(SEL)].filter(e => {
        const r = e.getBoundingClientRect(); const s = getComputedStyle(e);
        return r.width > 2 && r.height > 2 && s.visibility !== 'hidden' && s.display !== 'none' && !e.disabled;
      });
    },
    isText(e) {
      if (!e) return false;
      const t = e.tagName.toLowerCase();
      return t === 'textarea' || (t === 'input' && textTypes.includes((e.getAttribute('type') || '').toLowerCase()));
    },
    report() {
      const e = this.cur; if (!e || !window.ipc) return;
      const text = this.isText(e);
      window.ipc.postMessage(JSON.stringify({ kind: 'focus', text, password: (e.type || '') === 'password',
        value: text ? (e.value || '') : '',
        label: e.getAttribute('aria-label') || e.getAttribute('placeholder') || (e.labels && e.labels[0] && e.labels[0].innerText) || e.name || '' }));
    },
    mark(e) {
      if (this.cur && document.contains(this.cur)) this.cur.style.outline = this.saved;
      this.cur = e; this.saved = e.style.outline;
      e.style.outline = '3px solid ' + ACCENT; e.style.outlineOffset = '2px';
      e.focus({ preventScroll: true }); e.scrollIntoView({ block: 'nearest', inline: 'nearest' });
      this.report();
    },
    move(dir) {
      const items = this.items(); if (!items.length) return;
      if (!this.cur || !document.contains(this.cur) || !items.includes(this.cur)) {
        this.mark(items.find(e => this.isText(e)) || items[0]); return;
      }
      const c = this.cur.getBoundingClientRect(); const cx = c.left + c.width / 2, cy = c.top + c.height / 2;
      let best = null, score = 1e12;
      for (const e of items) {
        if (e === this.cur) continue;
        const r = e.getBoundingClientRect(); const x = r.left + r.width / 2, y = r.top + r.height / 2;
        let gap, off, overlap;
        if (dir === 'down' && y > cy + 1) { gap = r.top - c.bottom; off = Math.abs(x - cx); overlap = Math.min(r.right, c.right) - Math.max(r.left, c.left); }
        else if (dir === 'up' && y < cy - 1) { gap = c.top - r.bottom; off = Math.abs(x - cx); overlap = Math.min(r.right, c.right) - Math.max(r.left, c.left); }
        else if (dir === 'right' && x > cx + 1) { gap = r.left - c.right; off = Math.abs(y - cy); overlap = Math.min(r.bottom, c.bottom) - Math.max(r.top, c.top); }
        else if (dir === 'left' && x < cx - 1) { gap = c.left - r.right; off = Math.abs(y - cy); overlap = Math.min(r.bottom, c.bottom) - Math.max(r.top, c.top); }
        else continue;
        const s = Math.max(gap, 0) + off * (overlap > 0 ? 0.3 : 2);
        if (s < score) { score = s; best = e; }
      }
      if (best) this.mark(best);
      else if (dir === 'down') window.scrollBy(0, 240);
      else if (dir === 'up') window.scrollBy(0, -240);
    },
    activate() {
      const e = this.cur; if (!e || !document.contains(e)) { this.move('down'); return; }
      if (this.isText(e)) { this.report(); return; }
      e.click();
    },
    setValue(v) {
      const e = this.cur; if (!this.isText(e)) return;
      const proto = e.tagName.toLowerCase() === 'textarea' ? HTMLTextAreaElement.prototype : HTMLInputElement.prototype;
      Object.getOwnPropertyDescriptor(proto, 'value').set.call(e, v);
      e.dispatchEvent(new Event('input', { bubbles: true }));
      e.dispatchEvent(new Event('change', { bubbles: true }));
    },
    submit() {
      const e = this.cur; if (!e) return;
      if (e.form) { e.form.requestSubmit ? e.form.requestSubmit() : e.form.submit(); }
      else e.dispatchEvent(new KeyboardEvent('keydown', { key: 'Enter', code: 'Enter', keyCode: 13, bubbles: true }));
    },
  };
  const PAGE = "__PAGE__";
  const reportPage = () => {
    if (PAGE && location.href.startsWith(PAGE) && window.ipc)
      window.ipc.postMessage(JSON.stringify({ kind: 'page', url: location.href, body: document.body ? document.body.innerText : '' }));
  };
  if (document.readyState === 'loading') document.addEventListener('DOMContentLoaded', reportPage); else reportPage();
  document.addEventListener('focusin', ev => { if (ev.target !== cs.cur && ev.target.matches && ev.target.matches(SEL)) { cs.cur = ev.target; cs.report(); } }, true);
})();
"#
    .replace("__ACCENT__", accent)
    .replace("__PAGE__", page_prefix)
}

/// An open sign-in page.
pub struct WebLogin {
    view: WebView,
    // Keeps the WebView2 environment (and its per-profile data folder) alive with the page.
    _context: WebContext,
}

impl WebLogin {
    /// Opens `url` inside `window`. Redirects starting with `redirect_prefix` are not loaded; their query
    /// parameters are delivered as `WebEvent::Redirect`. `on_event` runs on the UI thread.
    /// `data_dir` holds cookies and is per profile, so profiles never share sign-ins. Pages whose URL
    /// starts with `page_prefix` (if not empty) are reported as `WebEvent::Page` with their text.
    pub fn open(
        window: &slint::Window,
        data_dir: PathBuf,
        url: &str,
        redirect_prefix: &str,
        page_prefix: &str,
        accent: &str,
        on_event: impl Fn(WebEvent) + Clone + Send + 'static,
    ) -> wry::Result<Self> {
        let handle = window.window_handle();
        let prefix = redirect_prefix.to_string();
        let nav_event = on_event.clone();
        let ipc_event = on_event;
        let mut context = WebContext::new(Some(data_dir));
        let view = WebViewBuilder::new_with_web_context(&mut context)
            .with_url(url)
            .with_initialization_script(page_script(accent, page_prefix))
            // Keep keyboard focus on Win2Console; the controller and keyboard drive the page via script.
            .with_focused(false)
            .with_devtools(false)
            .with_navigation_handler(move |nav| {
                if !prefix.is_empty() && nav.starts_with(&prefix) {
                    let params = query_params(&nav);
                    let ev = nav_event.clone();
                    let _ = slint::invoke_from_event_loop(move || ev(WebEvent::Redirect(params)));
                    false
                } else {
                    true
                }
            })
            .with_ipc_handler(move |req| {
                let Ok(v) = serde_json::from_str::<serde_json::Value>(req.body()) else { return };
                if v["kind"] == "page" {
                    let url = v["url"].as_str().unwrap_or_default().to_string();
                    let body = v["body"].as_str().unwrap_or_default().to_string();
                    let ev = ipc_event.clone();
                    let _ = slint::invoke_from_event_loop(move || ev(WebEvent::Page { url, body }));
                } else if v["kind"] == "focus" {
                    let info = FocusInfo {
                        text: v["text"].as_bool().unwrap_or(false),
                        password: v["password"].as_bool().unwrap_or(false),
                        value: v["value"].as_str().unwrap_or_default().to_string(),
                        label: v["label"].as_str().unwrap_or_default().trim().to_string(),
                    };
                    let ev = ipc_event.clone();
                    let _ = slint::invoke_from_event_loop(move || ev(WebEvent::Focus(info)));
                }
            })
            .build_as_child(&handle)?;
        let _ = view.focus_parent();
        Ok(Self { view, _context: context })
    }

    /// Places the page at a logical-pixel rectangle inside the window.
    pub fn set_bounds(&self, x: f32, y: f32, w: f32, h: f32) {
        let _ = self.view.set_bounds(wry::Rect {
            position: dpi::LogicalPosition::new(x as f64, y as f64).into(),
            size: dpi::LogicalSize::new(w.max(1.0) as f64, h.max(1.0) as f64).into(),
        });
    }

    fn run(&self, js: &str) {
        if let Err(e) = self.view.evaluate_script(js) {
            tracing::warn!("sign-in page script: {e}");
        }
    }

    pub fn nav(&self, dir: &str) {
        self.run(&format!("window.__cs && window.__cs.move('{dir}')"));
    }

    pub fn activate(&self) {
        self.run("window.__cs && window.__cs.activate()");
    }

    pub fn submit(&self) {
        self.run("window.__cs && window.__cs.submit()");
    }

    /// Replaces the focused field's text.
    pub fn set_value(&self, value: &str) {
        let json = serde_json::to_string(value).expect("string serialises");
        self.run(&format!("window.__cs && window.__cs.setValue({json})"));
    }

    pub fn back(&self) {
        self.run("history.length > 1 && history.back()");
    }

    /// `Cookie` header value with this page's cookies for `url` (empty if none).
    pub fn cookie_header(&self, url: &str) -> String {
        match self.view.cookies_for_url(url) {
            Ok(cookies) => cookies.iter().map(|c| format!("{}={}", c.name(), c.value())).collect::<Vec<_>>().join("; "),
            Err(e) => {
                tracing::warn!("reading sign-in cookies: {e}");
                String::new()
            }
        }
    }

    /// Gives keyboard focus back to the Win2Console window (so the controller/keyboard drive our UI).
    pub fn focus_parent(&self) {
        let _ = self.view.focus_parent();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn redirect_query_parsing() {
        let p = query_params("http://127.0.0.1/callback?code=abc%2F123&state=x+y#frag");
        assert_eq!(p, [("code".to_string(), "abc/123".to_string()), ("state".to_string(), "x y".to_string())]);
        assert!(query_params("http://127.0.0.1/callback").is_empty());
    }

    #[test]
    fn pkce_shapes() {
        let p = Pkce::new();
        assert_eq!(p.verifier.len(), 64); // 48 bytes -> 64 base64url chars
        assert_eq!(p.challenge.len(), 43); // SHA-256 -> 43 chars
        assert!(!p.verifier.contains(['+', '/', '=']));
        assert_ne!(Pkce::new().verifier, p.verifier);
    }

    #[test]
    fn encoding() {
        assert_eq!(encode("http://127.0.0.1/callback"), "http%3A%2F%2F127.0.0.1%2Fcallback");
        assert_eq!(encode("openid sdk.social_layer"), "openid%20sdk.social_layer");
    }
}
