// SSE line/frame parser — port of open-sse/utils/streamHelpers.ts normalizer.
// Handles: `data:` payload join, `event:`/`id:`/`retry:` fields, comment lines,
// ANSI escape pollution from terminal-redraw upstreams (#2273), multi-line
// data join with '\n', "[DONE]" sentinel.

use super::SseFrame;

/// Strip ANSI/VT100 escapes + stray C0 controls, preserving \t \n \r.
/// Bounded, O(n). Port of ANSI_ESCAPE_RE handling.
pub fn strip_ansi(v: &str) -> String {
    let mut out = String::with_capacity(v.len());
    let mut chars = v.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\x1b' {
            if let Some(next) = chars.peek().copied() {
                match next {
                    '[' => {
                        chars.next();
                        while let Some(&b) = chars.peek() {
                            let k = b as u32;
                            chars.next();
                            if (0x40..=0x7e).contains(&k) {
                                break;
                            }
                        }
                        continue;
                    }
                    ']' => {
                        chars.next();
                        // consume until BEL or ST(ESC \)
                        let mut done = false;
                        let mut prev = '\0';
                        let mut esc_next = false;
                        while let Some(&b) = chars.peek() {
                            chars.next();
                            if b == '\x07' {
                                done = true;
                                break;
                            }
                            if prev == '\x1b' && b == '\\' {
                                done = true;
                                break;
                            }
                            esc_next = b == '\x1b';
                            prev = if esc_next { '\0' } else { b };
                        }
                        if !done {
                            let _ = esc_next;
                        }
                        continue;
                    }
                    // single-char C1 file types
                    'A'..='Z' | '\\' | '^' | '_' | '`' => {
                        chars.next();
                        continue;
                    }
                    _ => continue,
                }
            }
            continue;
        }
        let k = c as u32;
        if k <= 0x1f && c != '\t' && c != '\n' && c != '\r' {
            continue;
        }
        out.push(c);
    }
    out
}

/// Streaming frame assembler: feed line-by-line, get completed SSE frames.
#[derive(Debug, Default, Clone)]
pub struct SseAssembler {
    pending_event: Option<String>,
    pending_data: Vec<String>,
}

impl SseAssembler {
    pub fn new() -> Self {
        Self::default()
    }

    /// Feed one raw line ('\n' stripped by caller or included; \r trimmed here).
    /// Returns a completed frame when a blank line closes the event.
    pub fn feed_line(&mut self, raw: &str) -> Option<SseFrame> {
        let line = strip_ansi(raw.trim_end_matches('\r'));
        let trimmed = line.trim_start();

        // Pure comment / field markers
        if trimmed.is_empty() {
            return self.flush();
        }
        if let Some(rest) = trimmed.strip_prefix("event:") {
            self.pending_event = Some(rest.trim().to_string());
            return None;
        }
        if trimmed.starts_with(':') {
            return None; // comment keepalive
        }
        if trimmed == "id:" || trimmed == "retry:" {
            return None;
        }
        if let Some(rest) = trimmed.strip_prefix("data:") {
            self.pending_data.push(rest.strip_prefix(' ').unwrap_or(rest).to_string());
            return None;
        }
        None
    }

    /// Force-close current pending frame (end-of-stream).
    pub fn finish(&mut self) -> Option<SseFrame> {
        self.flush()
    }

    fn flush(&mut self) -> Option<SseFrame> {
        if self.pending_data.is_empty() {
            self.pending_event = None;
            return None;
        }
        let data = self.pending_data.join("\n");
        let event = self.pending_event.take();
        self.pending_data.clear();
        let done = data.trim() == "[DONE]";
        Some(SseFrame {
            event,
            data,
            done,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn basic_frame() {
        let mut a = SseAssembler::new();
        assert!(a.feed_line("event: message_start").is_none());
        assert!(a.feed_line("data: {\"type\":\"message_start\"}").is_none());
        let f = a.feed_line("").unwrap();
        assert_eq!(f.event.as_deref(), Some("message_start"));
        assert!(f.data.contains("message_start"));
        assert!(!f.done);
    }

    #[test]
    fn openai_chunk_without_event_field() {
        let mut a = SseAssembler::new();
        assert!(a.feed_line("data: {\"choices\":[]}").is_none());
        let f = a.feed_line("").unwrap();
        assert!(f.event.is_none());
    }

    #[test]
    fn done_sentinel() {
        let mut a = SseAssembler::new();
        assert!(a.feed_line("data: [DONE]").is_none());
        let f = a.feed_line("").unwrap();
        assert!(f.done);
    }

    #[test]
    fn ansi_polluted_frame() {
        let mut a = SseAssembler::new();
        assert!(a.feed_line("\x1b[2K\x1b[1Adata: {\"ok\":true}").is_none());
        let f = a.feed_line("").unwrap();
        assert_eq!(f.data.trim_start(), "{\"ok\":true}");
    }

    #[test]
    fn multiline_data_joined() {
        let mut a = SseAssembler::new();
        assert!(a.feed_line("data: {\"a\":").is_none());
        assert!(a.feed_line("data: 1}").is_none());
        let f = a.feed_line("").unwrap();
        assert_eq!(f.data.trim(), "{\"a\":\n1}"); // join preserves content, strips one leading space
    }

    #[test]
    fn comments_skipped() {
        let mut a = SseAssembler::new();
        assert!(a.feed_line(": keepalive").is_none());
        assert!(a.feed_line("").is_none()); // nothing to flush
    }
}
