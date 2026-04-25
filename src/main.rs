use crust::{Crust, Pane, Input};
use crust::style;
use std::path::PathBuf;
use std::process::{Command, Stdio};

// All settable key=value lines tile understands.
const PALETTE_DEFAULT: &str = "#ff5555,#50fa7b,#bd93f9,#ffb86c,#8be9fd,#f1fa8c,#ff79c6,#9aedfe";

#[derive(Clone)]
enum ItemKind {
    HexColor(String),                       // #rrggbb
    Number(u32, u32, u32),                  // value, min, max
    Palette(Vec<String>),                   // comma-separated hex list
}

#[derive(Clone)]
struct Item {
    label: String,
    key: &'static str,
    help: &'static str,
    kind: ItemKind,
}

struct Category {
    name: String,
    items: Vec<Item>,
}

struct App {
    top: Pane,
    left: Pane,
    right: Pane,
    status: Pane,
    categories: Vec<Category>,
    cat_index: usize,
    item_index: usize,
    autostart: Vec<String>,                 // raw `exec ...` payloads
    binds: Vec<String>,                     // raw `bind ...` payloads
    pins: Vec<(u8, u8)>,                    // (ws, output)
    raw_lines: Vec<String>,                 // every original line (so save preserves order)
    dirty: bool,
    config_path: PathBuf,
}

impl App {
    fn new() -> Self {
        let home = std::env::var("HOME").unwrap_or_else(|_| "/tmp".into());
        let config_path = PathBuf::from(&home).join(".tilerc");
        let (cols, rows) = Crust::terminal_size();
        let split = 22u16;
        let lw = split - 1;
        let rx = split + 3;
        let rw = cols.saturating_sub(rx).saturating_sub(1);

        let mut app = App {
            top: Pane::new(1, 1, cols, 1, 0, 236),
            left: Pane::new(2, 3, lw, rows.saturating_sub(4), 255, 0),
            right: Pane::new(rx, 3, rw, rows.saturating_sub(4), 252, 0),
            status: Pane::new(1, rows, cols, 1, 252, 236),
            categories: Vec::new(),
            cat_index: 0,
            item_index: 0,
            autostart: Vec::new(),
            binds: Vec::new(),
            pins: Vec::new(),
            raw_lines: Vec::new(),
            dirty: false,
            config_path,
        };
        app.left.border = true;
        app.right.border = true;
        app.build_categories();
        app.load_config();
        app
    }

    fn build_categories(&mut self) {
        self.categories = vec![
            Category { name: "Bar".into(), items: vec![
                Item { label: "Bar height".into(),  key: "bar_height", help: "px tall (also tab/WS square edge)",
                       kind: ItemKind::Number(10, 4, 64) },
                Item { label: "Bar pad".into(),     key: "bar_pad",    help: "px before first tab / after last WS",
                       kind: ItemKind::Number(4, 0, 64) },
                Item { label: "Strip height".into(), key: "strip_height", help: "px reserved at top for status bar",
                       kind: ItemKind::Number(0, 0, 64) },
                Item { label: "Inner gap".into(),   key: "gap_inner",  help: "px around managed windows",
                       kind: ItemKind::Number(0, 0, 32) },
                Item { label: "Bar bg".into(),      key: "bar_bg",     help: "background of the row-of-squares strip",
                       kind: ItemKind::HexColor("#000000".into()) },
            ]},
            Category { name: "Workspaces".into(), items: {
                let mut v = vec![
                    Item { label: "WS active".into(),   key: "ws_active",
                           help: "default colour for the current workspace square (when no per-WS override)",
                           kind: ItemKind::HexColor("#ffffff".into()) },
                    Item { label: "WS populated".into(), key: "ws_populated",
                           help: "default colour for non-current populated workspaces (when no per-WS override)",
                           kind: ItemKind::HexColor("#555555".into()) },
                    Item { label: "WS dim %".into(), key: "ws_dim_factor",
                           help: "non-active WS brightness 0..100 (only applied when ws_color_N is set)",
                           kind: ItemKind::Number(70, 0, 100) },
                ];
                // Per-workspace colour overrides — ws_color_1 .. ws_color_10.
                // Each one is a fixed colour for that workspace's square; the
                // active WS draws it at full intensity, populated WSes dim it.
                // Empty (sentinel) means use the WS active/populated default.
                for n in 1..=10 {
                    let label = format!("WS {} colour", n);
                    let key_str = format!("ws_color_{}", n);
                    let key: &'static str = Box::leak(key_str.into_boxed_str());
                    v.push(Item {
                        label, key,
                        help: "fixed colour for this workspace's square (overrides WS active/populated)",
                        kind: ItemKind::HexColor("".into()),
                    });
                }
                v
            }},
            Category { name: "Tabs".into(), items: vec![
                Item { label: "Tab default".into(), key: "tab_default",
                       help: "newly-spawned (uncoloured) tab colour",
                       kind: ItemKind::HexColor("#555555".into()) },
                Item { label: "Tab dim %".into(),   key: "tab_dim_factor",
                       help: "inactive-tab brightness 0..100",
                       kind: ItemKind::Number(40, 0, 100) },
                Item { label: "Tab palette".into(), key: "tab_palette",
                       help: "colours that Mod4+c rotates through",
                       kind: ItemKind::Palette(
                           PALETTE_DEFAULT.split(',').map(|s| s.trim().to_string()).collect()) },
            ]},
            Category { name: "Border".into(), items: vec![
                Item { label: "Border width".into(), key: "border_width",
                       help: "px around every managed window (0 disables)",
                       kind: ItemKind::Number(1, 0, 16) },
                Item { label: "Focus colour".into(), key: "border_focused",
                       help: "border colour of the focused window",
                       kind: ItemKind::HexColor("#ffffff".into()) },
                Item { label: "Unfocus colour".into(), key: "border_unfocused",
                       help: "border colour of all other windows",
                       kind: ItemKind::HexColor("#222222".into()) },
            ]},
            Category { name: "Layout".into(), items: vec![
                Item { label: "Master ratio %".into(), key: "master_ratio",
                       help: "% of width given to master pane (10..90)",
                       kind: ItemKind::Number(50, 10, 90) },
            ]},
            // Autostart, Bindings, Pins are surfaced as informational
            // categories — full-featured editing happens in the file.
        ];
    }

    fn load_config(&mut self) {
        let content = match std::fs::read_to_string(&self.config_path) {
            Ok(c) => c, Err(_) => return,
        };
        for line in content.lines() {
            self.raw_lines.push(line.to_string());
            let trimmed = line.trim();
            if trimmed.is_empty() || trimmed.starts_with('#') { continue; }

            // exec <cmdline>
            if let Some(rest) = trimmed.strip_prefix("exec ") {
                self.autostart.push(rest.trim().to_string());
                continue;
            }
            // bind <chord> <action> [arg]
            if let Some(rest) = trimmed.strip_prefix("bind ") {
                self.binds.push(rest.trim().to_string());
                continue;
            }
            // pin <ws> <output>
            if let Some(rest) = trimmed.strip_prefix("pin ") {
                let mut parts = rest.split_whitespace();
                if let (Some(a), Some(b)) = (parts.next(), parts.next()) {
                    if let (Ok(ws), Ok(o)) = (a.parse::<u8>(), b.parse::<u8>()) {
                        self.pins.push((ws, o));
                    }
                }
                continue;
            }
            // key = value
            if let Some((k, v)) = trimmed.split_once('=') {
                let key = k.trim();
                let val = v.trim().split('#').next().unwrap_or("").trim();
                let val = if val.starts_with('#') { v.trim() } else { val };
                // Re-handle: only strip trailing inline `# ...` comments when
                // the value itself isn't a hex colour.
                let val = if let Some(idx) = v.find('#') {
                    let head = &v[..idx];
                    let tail = &v[idx..];
                    // keep the `#` if the next 6 chars are hex
                    if tail.len() >= 7 && tail[1..7].chars().all(|c| c.is_ascii_hexdigit()) {
                        v.trim()
                    } else {
                        head.trim()
                    }
                } else { val };
                self.update_item(key, val);
            }
        }
    }

    fn update_item(&mut self, key: &str, val: &str) {
        for cat in &mut self.categories {
            for item in &mut cat.items {
                if item.key != key { continue; }
                match &mut item.kind {
                    ItemKind::HexColor(s) => {
                        if let Some(n) = normalize_hex(val) { *s = n; }
                    }
                    ItemKind::Number(v, min, max) => {
                        if let Ok(n) = val.parse::<u32>() {
                            if n >= *min && n <= *max { *v = n; }
                        }
                    }
                    ItemKind::Palette(v) => {
                        let parsed: Vec<String> = val.split(',')
                            .map(|s| s.trim().to_string())
                            .filter(|s| normalize_hex(s).is_some())
                            .map(|s| normalize_hex(&s).unwrap())
                            .collect();
                        if !parsed.is_empty() { *v = parsed; }
                    }
                }
            }
        }
    }

    // Tell tile to re-parse ~/.tilerc and re-render. tile catches
    // SIGUSR1 → reload_runtime: re-grabs keys, re-paints borders,
    // re-applies the visible workspace's layout. No-op if tile isn't
    // running.
    fn reload_tile() {
        // -x = exact name match (so we don't hit `tileconf`).
        let _ = Command::new("pkill")
            .args(["-USR1", "-x", "tile"])
            .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null())
            .status();
    }

    // Strip has no SIGUSR1 path, so a config-affecting change requires
    // a full restart. setsid + redirected fds detach the new strip from
    // tileconf so it survives if tileconf exits. No-op if strip wasn't
    // running.
    // Show a save-confirmation + reload prompt. After save, ask:
    //   y → reload tile + restart strip
    //   t → reload tile only (cheap; avoids strip flicker)
    //   anything else → no reload
    // Status line communicates the chosen action.
    fn prompt_reload(status: &mut Pane) {
        status.say(&style::fg(" Saved. Reload? y=tile+strip  t=tile only  n=skip", 220));
        let Some(k) = Input::getchr(None) else { return; };
        match k.as_str() {
            "y" | "Y" => {
                Self::reload_tile();
                Self::restart_strip();
                status.say(&style::fg(" Saved + reloaded tile + restarted strip", 82));
            }
            "t" | "T" => {
                Self::reload_tile();
                status.say(&style::fg(" Saved + reloaded tile", 82));
            }
            _ => {
                status.say(&style::fg(" Saved (no reload)", 82));
            }
        }
        std::thread::sleep(std::time::Duration::from_millis(700));
    }

    fn restart_strip() {
        // -x = exact match (so we don't hit `stripconf` or any other process
        // whose name happens to contain "strip").
        let killed = Command::new("pkill")
            .args(["-x", "strip"])
            .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null())
            .status()
            .map(|s| s.success())
            .unwrap_or(false);
        if !killed { return; }
        // Brief pause so the kernel has time to reap before we re-spawn.
        std::thread::sleep(std::time::Duration::from_millis(100));
        let _ = Command::new("setsid")
            .arg("strip")
            .stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null())
            .spawn();
    }

    fn save_config(&self) {
        // Strategy: rewrite the file. Walk raw_lines; if a line is a
        // settable `key = value`, replace its value with the current
        // in-memory value (preserve indentation + comment). Otherwise
        // emit verbatim. Add any settings the file didn't have at the end.
        let mut written = std::collections::HashSet::new();
        let mut out = String::new();
        for line in &self.raw_lines {
            let trimmed = line.trim_start();
            let indent = &line[..line.len() - trimmed.len()];
            if let Some((k, _)) = trimmed.split_once('=') {
                let key = k.trim();
                // If the key is one we manage and its current value is
                // an "unset" sentinel (HexColor cleared to empty),
                // remove the line entirely so tile reverts to the
                // default. Track via written so the append pass below
                // doesn't re-emit it.
                if self.is_managed_key(key) && self.serialize_value(key).is_none() {
                    written.insert(key.to_string());
                    continue;
                }
                if let Some(val) = self.serialize_value(key) {
                    written.insert(key.to_string());
                    let comment = trimmed.find('#')
                        .filter(|&i| {
                            let tail = &trimmed[i..];
                            !(tail.len() >= 7 && tail[1..7].chars().all(|c| c.is_ascii_hexdigit()))
                        })
                        .map(|i| trimmed[i..].to_string());
                    let mut new_line = format!("{}{} = {}", indent, key, val);
                    if let Some(c) = comment {
                        new_line.push_str("  ");
                        new_line.push_str(&c);
                    }
                    out.push_str(&new_line);
                    out.push('\n');
                    continue;
                }
            }
            out.push_str(line);
            out.push('\n');
        }
        // Append anything the file never mentioned.
        let mut appended = false;
        for cat in &self.categories {
            for item in &cat.items {
                if written.contains(item.key) { continue; }
                if let Some(val) = self.serialize_value(item.key) {
                    if !appended {
                        out.push_str("\n# Added by tileconf\n");
                        appended = true;
                    }
                    out.push_str(&format!("{} = {}\n", item.key, val));
                }
            }
        }
        atomic_write(&self.config_path, out.as_bytes());
    }

    // Is `key` one of the settings tileconf knows about?
    fn is_managed_key(&self, key: &str) -> bool {
        for cat in &self.categories {
            for item in &cat.items {
                if item.key == key { return true; }
            }
        }
        false
    }

    fn serialize_value(&self, key: &str) -> Option<String> {
        for cat in &self.categories {
            for item in &cat.items {
                if item.key != key { continue; }
                return match &item.kind {
                    // Empty HexColor = "unset" sentinel. Don't emit the
                    // key=value line at all so tile sees no override
                    // (and falls back to ws_active / ws_populated).
                    ItemKind::HexColor(s) if s.is_empty() => None,
                    ItemKind::HexColor(s) => Some(s.clone()),
                    ItemKind::Number(v, _, _) => Some(v.to_string()),
                    ItemKind::Palette(v) => Some(v.join(",")),
                };
            }
        }
        None
    }

    // --- helpers ----------------------------------------------------

    fn current_color(&self, key: &str) -> Option<&str> {
        for cat in &self.categories {
            for item in &cat.items {
                if item.key == key {
                    if let ItemKind::HexColor(s) = &item.kind { return Some(s); }
                }
            }
        }
        None
    }

    fn fg24(text: &str, hex: &str) -> String {
        if let Some((r, g, b)) = parse_hex(hex) {
            format!("\x1b[38;2;{};{};{}m{}\x1b[0m", r, g, b, text)
        } else { text.to_string() }
    }
    fn bg24(text: &str, hex: &str) -> String {
        if let Some((r, g, b)) = parse_hex(hex) {
            format!("\x1b[48;2;{};{};{}m{}\x1b[0m", r, g, b, text)
        } else { text.to_string() }
    }

    fn category_count(&self) -> usize {
        self.categories.len() + 3                 // + Autostart + Bindings + Pins
    }

    fn current_items_len(&self) -> usize {
        if self.cat_index < self.categories.len() {
            self.categories[self.cat_index].items.len()
        } else {
            0
        }
    }

    fn category_name(&self, i: usize) -> String {
        if i < self.categories.len() {
            self.categories[i].name.clone()
        } else {
            match i - self.categories.len() {
                0 => "Autostart".into(),
                1 => "Bindings".into(),
                _ => "Pins".into(),
            }
        }
    }

    // --- render -----------------------------------------------------

    fn render(&mut self) {
        let dirty_mark = if self.dirty { " [modified]" } else { "" };
        let bar_bg = self.current_color("bar_bg").unwrap_or("#000000").to_string();
        let ws_a = self.current_color("ws_active").unwrap_or("#ffffff").to_string();
        let ws_p = self.current_color("ws_populated").unwrap_or("#555555").to_string();
        let preview = format!(" {}{}{}{}{}{} ",
            Self::bg24(&Self::fg24("\u{25A0}", &ws_p), &bar_bg),
            Self::bg24(" ", &bar_bg),
            Self::bg24(&Self::fg24("\u{25A0}", &ws_a), &bar_bg),
            Self::bg24(" ", &bar_bg),
            Self::bg24(&Self::fg24("\u{25A0}", &ws_p), &bar_bg),
            Self::bg24(" ", &bar_bg));
        self.top.say(&format!(" tileconf{}    bar:{}", dirty_mark, preview));

        let mut lines = Vec::new();
        for i in 0..self.category_count() {
            let name = self.category_name(i);
            if i == self.cat_index {
                lines.push(style::reverse(&format!(" {} ", name)));
            } else {
                lines.push(format!(" {} ", name));
            }
        }
        self.left.set_text(&lines.join("\n"));
        self.left.ix = 0;
        self.left.full_refresh();

        self.render_items();

        let len = self.current_items_len();
        let hint = if self.cat_index < self.categories.len() {
            "j/k:item J/K:cat h/l:adjust Enter:edit W/s:save q:quit"
        } else {
            "J/K:category — this list is read-only (edit ~/.tilerc)  q:quit"
        };
        self.status.say(&format!(" {}/{}  {}",
            self.item_index + 1, len.max(1), hint));
    }

    fn render_items(&mut self) {
        let mut lines = Vec::new();

        if self.cat_index >= self.categories.len() {
            // Informational tabs: autostart / bindings / pins.
            let which = self.cat_index - self.categories.len();
            let (title, body): (&str, Vec<String>) = match which {
                0 => ("Autostart (`exec ...`)",
                      self.autostart.iter().map(|s| format!("  exec {}", s)).collect()),
                1 => ("Bindings (`bind ...`)",
                      self.binds.iter().map(|s| format!("  bind {}", s)).collect()),
                _ => ("Pins (`pin <ws> <output>`)",
                      self.pins.iter().map(|(w, o)| format!("  pin {} {}", w, o)).collect()),
            };
            lines.push(style::fg(&style::bold(title), 81));
            lines.push(style::fg(&"\u{2500}".repeat(40), 245));
            lines.push(String::new());
            if body.is_empty() {
                lines.push(style::fg("  (none defined)", 245));
            } else {
                for l in body { lines.push(l); }
            }
            lines.push(String::new());
            lines.push(style::fg("Edit ~/.tilerc directly to change these.", 245));
            self.right.set_text(&lines.join("\n"));
            self.right.ix = 0;
            self.right.full_refresh();
            return;
        }

        let cat = &self.categories[self.cat_index];
        lines.push(style::fg(&style::bold(&cat.name), 81));
        lines.push(style::fg(&"\u{2500}".repeat(40), 245));
        lines.push(String::new());

        for (i, item) in cat.items.iter().enumerate() {
            let selected = i == self.item_index;
            let label = format!("{:<16}", item.label);
            let label = if selected { style::underline(&label) } else { label };
            let al = if selected { "\u{25C0} " } else { "  " };
            let ar = if selected { " \u{25B6}" } else { "  " };

            let val_str = match &item.kind {
                ItemKind::HexColor(hex) if hex.is_empty() => {
                    style::fg("(unset)", 245)
                }
                ItemKind::HexColor(hex) => {
                    let swatch = Self::bg24("    ", hex);
                    format!("{} {}", swatch, hex)
                }
                ItemKind::Number(v, _, _) => format!("{}", v),
                ItemKind::Palette(v) => {
                    let mut s = String::new();
                    for hex in v { s += &Self::bg24("  ", hex); }
                    if v.is_empty() { s = style::fg("(empty)", 245); }
                    format!("{} ({} colors)", s, v.len())
                }
            };
            lines.push(format!("  {}{}{}{}", label, al, val_str, ar));
        }

        // Help line for the focused item.
        if let Some(item) = cat.items.get(self.item_index) {
            lines.push(String::new());
            lines.push(style::fg(item.help, 245));
        }

        self.right.set_text(&lines.join("\n"));
        self.right.ix = 0;
        self.right.full_refresh();
    }

    // --- navigation -------------------------------------------------

    fn move_down(&mut self) {
        let len = self.current_items_len();
        if self.item_index + 1 < len { self.item_index += 1; }
    }
    fn move_up(&mut self) {
        if self.item_index > 0 { self.item_index -= 1; }
    }
    fn next_category(&mut self) {
        if self.cat_index + 1 < self.category_count() {
            self.cat_index += 1; self.item_index = 0;
        }
    }
    fn prev_category(&mut self) {
        if self.cat_index > 0 { self.cat_index -= 1; self.item_index = 0; }
    }

    fn next_value(&mut self) {
        if self.cat_index >= self.categories.len() { return; }
        if let Some(item) = self.categories[self.cat_index].items.get_mut(self.item_index) {
            if let ItemKind::Number(v, _, max) = &mut item.kind {
                if *v < *max { *v += 1; self.dirty = true; }
            }
        }
    }
    fn prev_value(&mut self) {
        if self.cat_index >= self.categories.len() { return; }
        if let Some(item) = self.categories[self.cat_index].items.get_mut(self.item_index) {
            if let ItemKind::Number(v, min, _) = &mut item.kind {
                if *v > *min { *v -= 1; self.dirty = true; }
            }
        }
    }

    fn edit_value(&mut self) {
        if self.cat_index >= self.categories.len() { return; }
        let (kind_label, initial) = {
            let item = match self.categories[self.cat_index].items.get(self.item_index) {
                Some(i) => i, None => return,
            };
            let init = match &item.kind {
                ItemKind::HexColor(s) => s.clone(),
                ItemKind::Number(v, _, _) => v.to_string(),
                ItemKind::Palette(v) => v.join(","),
            };
            (item.label.clone(), init)
        };

        let orig_bg = self.status.bg;
        self.status.bg = 18;
        let new_val = self.status.ask(&format!("{}: ", kind_label), &initial);
        self.status.bg = orig_bg;
        let new_val = new_val.trim().to_string();
        if new_val.is_empty() { return; }

        let item = &mut self.categories[self.cat_index].items[self.item_index];
        match &mut item.kind {
            ItemKind::HexColor(s) => {
                if let Some(n) = normalize_hex(&new_val) {
                    *s = n; self.dirty = true;
                }
            }
            ItemKind::Number(v, min, max) => {
                if let Ok(n) = new_val.parse::<u32>() {
                    if n >= *min && n <= *max { *v = n; self.dirty = true; }
                }
            }
            ItemKind::Palette(v) => {
                let parsed: Vec<String> = new_val.split(',')
                    .map(|s| s.trim().to_string())
                    .filter(|s| normalize_hex(s).is_some())
                    .map(|s| normalize_hex(&s).unwrap())
                    .collect();
                if !parsed.is_empty() { *v = parsed; self.dirty = true; }
            }
        }
    }
}

// Atomic file replace: write to PATH.tmp, rename PATH→PATH.bak, rename
// PATH.tmp→PATH. Guarantees PATH is never left empty/truncated even if
// the process is killed mid-save: PATH.bak is the previous good copy.
fn atomic_write(path: &std::path::Path, data: &[u8]) {
    use std::ffi::OsString;
    let mut tmp: OsString = path.as_os_str().to_owned();
    tmp.push(".tmp");
    let mut bak: OsString = path.as_os_str().to_owned();
    bak.push(".bak");
    if std::fs::write(&tmp, data).is_err() { return; }
    let _ = std::fs::rename(path, &bak);
    let _ = std::fs::rename(&tmp, path);
}

fn parse_hex(hex: &str) -> Option<(u8, u8, u8)> {
    let h = hex.trim().trim_start_matches('#');
    if h.len() != 6 { return None; }
    let r = u8::from_str_radix(&h[0..2], 16).ok()?;
    let g = u8::from_str_radix(&h[2..4], 16).ok()?;
    let b = u8::from_str_radix(&h[4..6], 16).ok()?;
    Some((r, g, b))
}
fn normalize_hex(hex: &str) -> Option<String> {
    parse_hex(hex).map(|(r, g, b)| format!("#{:02x}{:02x}{:02x}", r, g, b))
}

fn main() {
    Crust::init();
    let mut app = App::new();
    app.left.border_refresh();
    app.right.border_refresh();
    app.render();

    loop {
        let Some(key) = Input::getchr(None) else { continue };
        match key.as_str() {
            "q" | "ESC" => {
                if app.dirty {
                    app.status.say(&style::fg(" Save changes? (y/n)", 220));
                    if let Some(k) = Input::getchr(None) {
                        if k == "y" || k == "Y" {
                            app.save_config();
                            App::prompt_reload(&mut app.status);
                        }
                    }
                }
                break;
            }
            "j" | "DOWN" => { app.move_down(); app.render(); }
            "k" | "UP" => { app.move_up(); app.render(); }
            "J" | "PgDOWN" => { app.next_category(); app.render(); }
            "K" | "PgUP" => { app.prev_category(); app.render(); }
            "l" | "RIGHT" | "TAB" => { app.next_value(); app.render(); }
            "h" | "LEFT" | "S-TAB" => { app.prev_value(); app.render(); }
            "ENTER" => { app.edit_value(); app.render(); }
            "W" | "s" => {
                app.save_config();
                app.dirty = false;
                App::prompt_reload(&mut app.status);
                app.render();
            }
            "RESIZE" => {
                let (cols, rows) = Crust::terminal_size();
                let split = 22u16;
                let lw = split - 1;
                let rx = split + 3;
                let rw = cols.saturating_sub(rx).saturating_sub(1);
                app.top = Pane::new(1, 1, cols, 1, 0, 236);
                app.left = Pane::new(2, 3, lw, rows.saturating_sub(4), 255, 0);
                app.right = Pane::new(rx, 3, rw, rows.saturating_sub(4), 252, 0);
                app.status = Pane::new(1, rows, cols, 1, 252, 236);
                app.left.border = true;
                app.right.border = true;
                Crust::clear_screen();
                app.left.border_refresh();
                app.right.border_refresh();
                app.render();
            }
            _ => {}
        }
    }

    Crust::cleanup();
}
