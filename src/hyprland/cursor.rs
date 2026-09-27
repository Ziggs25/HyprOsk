use serde::Deserialize;
use std::io::{Read, Write};
use std::os::unix::net::UnixStream;
use std::path::PathBuf;
use std::time::{Duration, Instant};

/// Information about a monitor queried from Hyprland IPC (`j/monitors`).
#[derive(Debug, Clone, Deserialize, PartialEq)]
pub struct HyprMonitorInfo {
    pub id: Option<i64>,
    pub name: Option<String>,
    #[serde(default)]
    pub x: i32,
    #[serde(default)]
    pub y: i32,
    #[serde(default)]
    pub width: i32,
    #[serde(default)]
    pub height: i32,
    #[serde(default = "default_scale")]
    pub scale: f64,
    #[serde(default)]
    pub transform: i32,
    #[serde(default)]
    pub focused: bool,
    #[serde(default)]
    pub disabled: bool,
}

fn default_scale() -> f64 {
    1.0
}

impl HyprMonitorInfo {
    /// Return the logical bounding rectangle `(x, y, width, height)` in compositor coordinates.
    /// Accounts for monitor transformation (rotation swaps width/height) and fractional scaling.
    pub fn logical_rect(&self) -> (f64, f64, f64, f64) {
        // Transforms 1 (90°), 3 (270°), 5 (flipped 90°), 7 (flipped 270°) swap width & height
        let (pw, ph) = if self.transform % 2 != 0 {
            (self.height as f64, self.width as f64)
        } else {
            (self.width as f64, self.height as f64)
        };
        let scale = if self.scale > 0.0 { self.scale } else { 1.0 };
        let lw = pw / scale;
        let lh = ph / scale;
        (self.x as f64, self.y as f64, lw, lh)
    }

    /// True if the given global point is within this monitor's logical bounding box.
    pub fn contains_point(&self, x: f64, y: f64) -> bool {
        let (mx, my, mw, mh) = self.logical_rect();
        x >= mx && x <= mx + mw && y >= my && y <= my + mh
    }

    /// Calculate the relative vertical position (0.0 at top, 1.0 at bottom) within this monitor.
    pub fn relative_y(&self, y: f64) -> f64 {
        let (_, my, _, mh) = self.logical_rect();
        if mh > 0.0 {
            (y - my) / mh
        } else {
            0.5
        }
    }

    /// True if the given vertical coordinate is in the upper half (rel_y < 0.5) of this monitor.
    pub fn is_upper_half(&self, y: f64) -> bool {
        self.relative_y(y) < 0.5
    }
}

/// Distance from point `(px, py)` to axis-aligned rectangle `(rx, ry, rw, rh)`.
pub fn point_to_rect_distance(px: f64, py: f64, (rx, ry, rw, rh): (f64, f64, f64, f64)) -> f64 {
    let dx = if px < rx {
        rx - px
    } else if px > rx + rw {
        px - (rx + rw)
    } else {
        0.0
    };
    let dy = if py < ry {
        ry - py
    } else if py > ry + rh {
        py - (ry + rh)
    } else {
        0.0
    };
    (dx * dx + dy * dy).sqrt()
}

/// Find the active monitor containing the given point.
///
/// If the cursor coordinates do not lie inside any active monitor rectangle,
/// returns `None` so that HyprOsk treats inference as unavailable and safely
/// falls back to configured exclusivity instead of guessing an arbitrary monitor.
pub fn find_monitor_for_point<'a>(
    monitors: &'a [HyprMonitorInfo],
    x: f64,
    y: f64,
) -> Option<&'a HyprMonitorInfo> {
    if !x.is_finite() || !y.is_finite() {
        return None;
    }
    monitors
        .iter()
        .filter(|m| !m.disabled && m.width > 0 && m.height > 0)
        .find(|m| m.contains_point(x, y))
}

/// Parse cursor coordinates from Hyprland response (JSON `{"x": ..., "y": ...}` or plain `"X, Y"`).
/// Rejects non-finite coordinates (`NaN`, `Infinity`, etc.).
pub fn parse_cursor_pos(reply: &str) -> Option<(f64, f64)> {
    let trimmed = reply.trim();
    if let Ok(v) = serde_json::from_str::<serde_json::Value>(trimmed) {
        if let (Some(x), Some(y)) = (
            v.get("x").and_then(|x| x.as_f64().or_else(|| x.as_i64().map(|n| n as f64))),
            v.get("y").and_then(|y| y.as_f64().or_else(|| y.as_i64().map(|n| n as f64))),
        ) {
            if x.is_finite() && y.is_finite() {
                return Some((x, y));
            }
            return None;
        }
    }
    // Fallback: plain format "720, 480"
    if let Some((xs, ys)) = trimmed.split_once(',') {
        if let (Ok(x), Ok(y)) = (xs.trim().parse::<f64>(), ys.trim().parse::<f64>()) {
            if x.is_finite() && y.is_finite() {
                return Some((x, y));
            }
            return None;
        }
    }
    None
}

/// Parse monitor list from Hyprland JSON response (`j/monitors`).
pub fn parse_monitors(reply: &str) -> Option<Vec<HyprMonitorInfo>> {
    let trimmed = reply.trim();
    let monitors: Vec<HyprMonitorInfo> = serde_json::from_str(trimmed).ok()?;
    if monitors.is_empty() {
        None
    } else {
        Some(monitors)
    }
}

/// Classification of cursor relative vertical half on its monitor.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CursorVerticalPosition {
    UpperHalf,
    LowerHalf,
}

/// Result of best-effort cursor position inference.
#[derive(Debug, Clone, PartialEq)]
pub struct CursorInferenceResult {
    pub position: CursorVerticalPosition,
    pub cursor: (f64, f64),
    pub monitor_name: String,
    pub monitor_rect: (f64, f64, f64, f64),
    pub rel_y: f64,
}

/// Classify cursor position against a list of monitors.
pub fn classify_cursor_placement(
    monitors: &[HyprMonitorInfo],
    cursor: (f64, f64),
) -> Option<CursorInferenceResult> {
    if !cursor.0.is_finite() || !cursor.1.is_finite() {
        return None;
    }
    let monitor = find_monitor_for_point(monitors, cursor.0, cursor.1)?;
    let rel_y = monitor.relative_y(cursor.1);
    let position = if rel_y < 0.5 {
        CursorVerticalPosition::UpperHalf
    } else {
        CursorVerticalPosition::LowerHalf
    };
    Some(CursorInferenceResult {
        position,
        cursor,
        monitor_name: monitor.name.clone().unwrap_or_else(|| "unknown".to_string()),
        monitor_rect: monitor.logical_rect(),
        rel_y,
    })
}

/// Helper to resolve exclusivity zone based on cursor inference result or fallback to configured exclusivity.
pub fn resolve_placement_exclusivity(
    inference: Option<&CursorInferenceResult>,
    configured_exclusive: bool,
    height: u32,
) -> (bool, i32) {
    if let Some(res) = inference {
        let is_upper = res.position == CursorVerticalPosition::UpperHalf;
        let exclusive = !is_upper;
        let zone = if exclusive { height as i32 } else { 0 };
        (exclusive, zone)
    } else {
        let exclusive = configured_exclusive;
        let zone = if exclusive { height as i32 } else { 0 };
        (exclusive, zone)
    }
}

/// Send a request to Hyprland's request socket (`.socket.sock`) with a strict overall deadline.
pub fn query_hyprland_socket_with_deadline(request: &str, deadline: Instant) -> Option<String> {
    let now = Instant::now();
    if now >= deadline {
        return None;
    }
    let remaining = deadline.saturating_duration_since(now);
    let socket_timeout = remaining.min(Duration::from_millis(50));

    let xdg_runtime = std::env::var("XDG_RUNTIME_DIR").ok()?;
    let his = std::env::var("HYPRLAND_INSTANCE_SIGNATURE").ok()?;
    let path = PathBuf::from(xdg_runtime)
        .join("hypr")
        .join(his)
        .join(".socket.sock");
    if !path.exists() {
        return None;
    }

    let mut stream = UnixStream::connect(&path).ok()?;
    stream.set_write_timeout(Some(socket_timeout)).ok()?;
    stream.set_read_timeout(Some(socket_timeout)).ok()?;
    stream.write_all(request.as_bytes()).ok()?;

    let mut buf = Vec::with_capacity(8192);
    let mut tmp = [0u8; 4096];
    loop {
        match stream.read(&mut tmp) {
            Ok(0) => break,
            Ok(n) => {
                buf.extend_from_slice(&tmp[..n]);
                if buf.len() > 262144 {
                    break;
                }
            }
            Err(_) => break,
        }
        if Instant::now() >= deadline {
            break;
        }
    }
    if buf.is_empty() {
        None
    } else {
        Some(String::from_utf8_lossy(&buf).to_string())
    }
}

pub fn query_hyprland_socket(request: &str) -> Option<String> {
    let deadline = Instant::now() + Duration::from_millis(100);
    query_hyprland_socket_with_deadline(request, deadline)
}

/// Perform best-effort cursor-position inference by querying Hyprland IPC.
///
/// Bounded by a combined 100ms total deadline across both IPC queries.
/// Returns `Some(CursorInferenceResult)` if the cursor position and containing
/// monitor could be successfully retrieved and validated, or `None` if Hyprland
/// is unreachable, sockets time out, coordinates are non-finite or outside all active monitors.
pub fn infer_cursor_position_placement() -> Option<CursorInferenceResult> {
    let deadline = Instant::now() + Duration::from_millis(100);
    let cursor_reply = query_hyprland_socket_with_deadline("j/cursorpos", deadline)?;
    let cursor = parse_cursor_pos(&cursor_reply)?;

    let monitors_reply = query_hyprland_socket_with_deadline("j/monitors", deadline)?;
    let monitors = parse_monitors(&monitors_reply)?;

    classify_cursor_placement(&monitors, cursor)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_monitor(name: &str, x: i32, y: i32, width: i32, height: i32, scale: f64) -> HyprMonitorInfo {
        HyprMonitorInfo {
            id: Some(0),
            name: Some(name.to_string()),
            x,
            y,
            width,
            height,
            scale,
            transform: 0,
            focused: true,
            disabled: false,
        }
    }

    #[test]
    fn test_single_monitor_upper_and_lower_half() {
        let monitors = vec![sample_monitor("eDP-1", 0, 0, 1920, 1080, 1.0)];

        // Upper half (y < 540)
        let upper = classify_cursor_placement(&monitors, (500.0, 200.0)).expect("valid inference");
        assert_eq!(upper.position, CursorVerticalPosition::UpperHalf);
        assert!(upper.rel_y < 0.5);

        // Lower half (y >= 540)
        let lower = classify_cursor_placement(&monitors, (500.0, 800.0)).expect("valid inference");
        assert_eq!(lower.position, CursorVerticalPosition::LowerHalf);
        assert!(lower.rel_y >= 0.5);

        // Exact midpoint is classified as lower half
        let mid = classify_cursor_placement(&monitors, (500.0, 540.0)).expect("valid inference");
        assert_eq!(mid.position, CursorVerticalPosition::LowerHalf);
    }

    #[test]
    fn test_multi_monitor_side_by_side() {
        let monitors = vec![
            sample_monitor("eDP-1", 0, 0, 1920, 1080, 1.0),
            sample_monitor("HDMI-A-1", 1920, 0, 1920, 1080, 1.0),
        ];

        // Cursor on monitor 2, upper half
        let m2_upper = classify_cursor_placement(&monitors, (2500.0, 200.0)).expect("valid inference");
        assert_eq!(m2_upper.monitor_name, "HDMI-A-1");
        assert_eq!(m2_upper.position, CursorVerticalPosition::UpperHalf);

        // Cursor on monitor 2, lower half
        let m2_lower = classify_cursor_placement(&monitors, (2500.0, 900.0)).expect("valid inference");
        assert_eq!(m2_lower.monitor_name, "HDMI-A-1");
        assert_eq!(m2_lower.position, CursorVerticalPosition::LowerHalf);
    }

    #[test]
    fn test_negative_origins_vertical_stack() {
        // Monitor Top placed at negative Y: y = -1080, height = 1080 (range: -1080 to 0)
        // Monitor Bottom placed at y = 0, height = 1080 (range: 0 to 1080)
        let monitors = vec![
            sample_monitor("TOP-1", 0, -1080, 1920, 1080, 1.0),
            sample_monitor("BOTTOM-1", 0, 0, 1920, 1080, 1.0),
        ];

        // On TOP-1: midpoint is -1080 + 540 = -540
        // y = -800 is upper half (-800 < -540)
        let top_upper = classify_cursor_placement(&monitors, (500.0, -800.0)).expect("valid inference");
        assert_eq!(top_upper.monitor_name, "TOP-1");
        assert_eq!(top_upper.position, CursorVerticalPosition::UpperHalf);

        // y = -200 is lower half (-200 >= -540)
        let top_lower = classify_cursor_placement(&monitors, (500.0, -200.0)).expect("valid inference");
        assert_eq!(top_lower.monitor_name, "TOP-1");
        assert_eq!(top_lower.position, CursorVerticalPosition::LowerHalf);
    }

    #[test]
    fn test_negative_origins_horizontal_stack() {
        // Monitor Left placed at x = -1920, y = 0
        let monitors = vec![
            sample_monitor("LEFT-1", -1920, 0, 1920, 1080, 1.0),
            sample_monitor("MAIN-1", 0, 0, 1920, 1080, 1.0),
        ];

        let left_upper = classify_cursor_placement(&monitors, (-1000.0, 200.0)).expect("valid inference");
        assert_eq!(left_upper.monitor_name, "LEFT-1");
        assert_eq!(left_upper.position, CursorVerticalPosition::UpperHalf);
    }

    #[test]
    fn test_fractional_scale_and_transform() {
        // 1920x1280 with scale 1.3333334 -> logical 1440x960
        let mut m = sample_monitor("SCALED", 0, 0, 1920, 1280, 1.3333334);
        let (_x, _y, w, h) = m.logical_rect();
        assert!((w - 1440.0).abs() < 1.0);
        assert!((h - 960.0).abs() < 1.0);

        let monitors = vec![m.clone()];
        // y = 400 is upper half of 960 (400 < 480)
        let upper = classify_cursor_placement(&monitors, (720.0, 400.0)).expect("valid inference");
        assert_eq!(upper.position, CursorVerticalPosition::UpperHalf);

        // Transform 1 (90° rotated) swaps width and height: 1080x1920 logical
        m.width = 1920;
        m.height = 1080;
        m.scale = 1.0;
        m.transform = 1;
        let (_, _, rw, rh) = m.logical_rect();
        assert_eq!(rw, 1080.0);
        assert_eq!(rh, 1920.0);
        let rot_monitors = vec![m];
        // y = 800 is upper half of 1920 (800 < 960)
        let rot_upper = classify_cursor_placement(&rot_monitors, (500.0, 800.0)).expect("valid inference");
        assert_eq!(rot_upper.position, CursorVerticalPosition::UpperHalf);
    }

    #[test]
    fn test_cursor_outside_all_monitors_treated_as_unavailable() {
        let monitors = vec![sample_monitor("eDP-1", 0, 0, 1920, 1080, 1.0)];

        // Point outside the active monitor rectangle (e.g. 2500, 200)
        assert!(classify_cursor_placement(&monitors, (2500.0, 200.0)).is_none());
        // Point with negative coordinates outside
        assert!(classify_cursor_placement(&monitors, (-50.0, 200.0)).is_none());
    }

    #[test]
    fn test_fallback_on_empty_or_disabled_monitors() {
        assert!(classify_cursor_placement(&[], (500.0, 500.0)).is_none());

        let mut disabled = sample_monitor("DIS", 0, 0, 1920, 1080, 1.0);
        disabled.disabled = true;
        assert!(classify_cursor_placement(&[disabled], (500.0, 500.0)).is_none());
    }

    #[test]
    fn test_parse_cursor_pos_json_and_plain() {
        assert_eq!(parse_cursor_pos(r#"{"x": 720, "y": 480}"#), Some((720.0, 480.0)));
        assert_eq!(parse_cursor_pos("720, 480"), Some((720.0, 480.0)));
        assert_eq!(parse_cursor_pos(" -500 , -250 "), Some((-500.0, -250.0)));
        assert_eq!(parse_cursor_pos("invalid"), None);
    }

    #[test]
    fn test_parse_cursor_pos_rejects_non_finite() {
        assert_eq!(parse_cursor_pos(r#"{"x": null, "y": 480}"#), None);
        assert_eq!(parse_cursor_pos("NaN, 480"), None);
        assert_eq!(parse_cursor_pos("Infinity, 480"), None);
        assert_eq!(parse_cursor_pos("-Infinity, 480"), None);
        assert_eq!(parse_cursor_pos("720, NaN"), None);
    }

    #[test]
    fn test_resolve_placement_exclusivity() {
        let height = 300u32;
        // Unavailable inference falls back to configured exclusivity (both true and false)
        assert_eq!(resolve_placement_exclusivity(None, true, height), (true, 300));
        assert_eq!(resolve_placement_exclusivity(None, false, height), (false, 0));

        // Available inference: UpperHalf -> floating overlay (false, 0)
        let upper_res = CursorInferenceResult {
            position: CursorVerticalPosition::UpperHalf,
            cursor: (500.0, 200.0),
            monitor_name: "eDP-1".to_string(),
            monitor_rect: (0.0, 0.0, 1920.0, 1080.0),
            rel_y: 0.18,
        };
        assert_eq!(resolve_placement_exclusivity(Some(&upper_res), true, height), (false, 0));
        assert_eq!(resolve_placement_exclusivity(Some(&upper_res), false, height), (false, 0));

        // Available inference: LowerHalf -> docked (true, height)
        let lower_res = CursorInferenceResult {
            position: CursorVerticalPosition::LowerHalf,
            cursor: (500.0, 800.0),
            monitor_name: "eDP-1".to_string(),
            monitor_rect: (0.0, 0.0, 1920.0, 1080.0),
            rel_y: 0.74,
        };
        assert_eq!(resolve_placement_exclusivity(Some(&lower_res), true, height), (true, 300));
        assert_eq!(resolve_placement_exclusivity(Some(&lower_res), false, height), (true, 300));
    }

    #[test]
    fn test_parse_monitors_valid_and_malformed() {
        let valid_json = r#"[{"id":0,"name":"eDP-1","x":0,"y":0,"width":1920,"height":1080,"scale":1.0,"transform":0,"focused":true,"disabled":false}]"#;
        let monitors = parse_monitors(valid_json).expect("valid monitors json");
        assert_eq!(monitors.len(), 1);
        assert_eq!(monitors[0].name.as_deref(), Some("eDP-1"));

        assert_eq!(parse_monitors("[]"), None);
        assert_eq!(parse_monitors("invalid json"), None);
    }
}
