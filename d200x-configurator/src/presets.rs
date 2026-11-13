//! Ready-made pages for common applications. A preset creates a page with hotkeys for the
//! application and sets the page's `app` pattern so the deck switches to it automatically
//! whenever that application is in the foreground (needs the agent).

/// One key of a preset: label, Material icon, hotkey (as deck key names).
pub struct PresetKey {
    pub label: &'static str,
    pub icon: &'static str,
    pub keys: &'static [&'static str],
}

pub struct AppPreset {
    /// Display name in the library.
    pub name: &'static str,
    /// Page name created on the deck.
    pub page: &'static str,
    /// Foreground process names (any of these, case-insensitive substring).
    pub apps: &'static str,
    /// Icon for the "Go to page" key.
    pub icon: &'static str,
    /// Tile colour for the page's keys.
    pub color: &'static str,
    pub keys: &'static [PresetKey],
}

const fn k(label: &'static str, icon: &'static str, keys: &'static [&'static str]) -> PresetKey {
    PresetKey { label, icon, keys }
}

pub const APP_PRESETS: &[AppPreset] = &[
    AppPreset {
        name: "VS Code",
        page: "vscode",
        apps: "code.exe|code - insiders.exe|cursor.exe",
        icon: "microsoft-visual-studio-code",
        color: "#1f6feb",
        keys: &[
            k("Palette", "console-line", &["ctrl", "shift", "p"]),
            k("Find file", "file-search", &["ctrl", "p"]),
            k("Search", "text-search", &["ctrl", "shift", "f"]),
            k("Terminal", "console", &["ctrl", "`"]),
            k("Sidebar", "page-layout-sidebar-left", &["ctrl", "b"]),
            k("Format", "format-align-left", &["shift", "alt", "f"]),
            k("Comment", "comment-text", &["ctrl", "/"]),
            k("Go to def", "target", &["f12"]),
            k("Rename", "rename-box", &["f2"]),
            k("Run", "play", &["f5"]),
            k("Step over", "debug-step-over", &["f10"]),
            k("Step into", "debug-step-into", &["f11"]),
            k("Zen", "fullscreen", &["ctrl", "k", "z"]),
        ],
    },
    AppPreset {
        name: "Photoshop",
        page: "photoshop",
        apps: "photoshop.exe",
        icon: "image-edit",
        color: "#001e36",
        keys: &[
            k("Move", "cursor-move", &["v"]),
            k("Brush", "brush", &["b"]),
            k("Eraser", "eraser", &["e"]),
            k("Lasso", "vector-curve", &["l"]),
            k("Crop", "crop", &["c"]),
            k("Text", "format-text", &["t"]),
            k("Undo", "undo", &["ctrl", "z"]),
            k("Redo", "redo", &["ctrl", "shift", "z"]),
            k("New layer", "layers-plus", &["ctrl", "shift", "n"]),
            k("Merge down", "layers-triple", &["ctrl", "e"]),
            k("Deselect", "selection-off", &["ctrl", "d"]),
            k("Fit screen", "fit-to-screen", &["ctrl", "0"]),
            k("Export", "export", &["ctrl", "alt", "shift", "w"]),
        ],
    },
    AppPreset {
        name: "Lightroom",
        page: "lightroom",
        apps: "lightroom.exe",
        icon: "camera-iris",
        color: "#0f1c33",
        keys: &[
            k("Library", "view-grid", &["g"]),
            k("Develop", "tune", &["d"]),
            k("Loupe", "magnify", &["e"]),
            k("Compare", "compare", &["c"]),
            k("Flag", "flag", &["p"]),
            k("Reject", "flag-off", &["x"]),
            k("5 stars", "star", &["5"]),
            k("Crop", "crop", &["r"]),
            k("Before/After", "compare-horizontal", &["\\"]),
            k("Copy settings", "content-copy", &["ctrl", "shift", "c"]),
            k("Paste settings", "content-paste", &["ctrl", "shift", "v"]),
            k("Previous", "arrow-left", &["left"]),
            k("Next", "arrow-right", &["right"]),
        ],
    },
    AppPreset {
        name: "Premiere Pro",
        page: "premiere",
        apps: "adobe premiere pro.exe|premiere",
        icon: "movie-edit",
        color: "#2a0d3a",
        keys: &[
            k("Select", "cursor-default", &["v"]),
            k("Razor", "box-cutter", &["c"]),
            k("Ripple del", "delete-sweep", &["shift", "delete"]),
            k("Mark in", "ray-start", &["i"]),
            k("Mark out", "ray-end", &["o"]),
            k("Add edit", "content-cut", &["ctrl", "k"]),
            k("Play/Stop", "play-pause", &["space"]),
            k("Prev edit", "skip-previous", &["up"]),
            k("Next edit", "skip-next", &["down"]),
            k("Marker", "map-marker", &["m"]),
            k("Zoom fit", "fit-to-page", &["\\"]),
            k("Save", "content-save", &["ctrl", "s"]),
            k("Export", "export", &["ctrl", "m"]),
        ],
    },
    AppPreset {
        name: "DaVinci Resolve",
        page: "resolve",
        apps: "resolve.exe",
        icon: "filmstrip",
        color: "#1a1a1a",
        keys: &[
            k("Media", "folder-multiple-image", &["shift", "2"]),
            k("Cut", "content-cut", &["shift", "3"]),
            k("Edit", "movie-open", &["shift", "4"]),
            k("Color", "palette", &["shift", "6"]),
            k("Fairlight", "waveform", &["shift", "7"]),
            k("Deliver", "export", &["shift", "8"]),
            k("Blade", "box-cutter", &["ctrl", "b"]),
            k("Mark in", "ray-start", &["i"]),
            k("Mark out", "ray-end", &["o"]),
            k("Play/Stop", "play-pause", &["space"]),
            k("Undo", "undo", &["ctrl", "z"]),
            k("Marker", "map-marker", &["m"]),
            k("Save", "content-save", &["ctrl", "s"]),
        ],
    },
    AppPreset {
        name: "Blender",
        page: "blender",
        apps: "blender.exe",
        icon: "blender-software",
        color: "#e87d0d",
        keys: &[
            k("Move", "cursor-move", &["g"]),
            k("Rotate", "rotate-3d", &["r"]),
            k("Scale", "arrow-expand-all", &["s"]),
            k("Extrude", "arrow-up-bold-box", &["e"]),
            k("Edit mode", "vector-polygon", &["tab"]),
            k("Loop cut", "vector-intersection", &["ctrl", "r"]),
            k("Render", "camera", &["f12"]),
            k("Front", "axis-z-arrow", &["numpad1"]),
            k("Top", "axis-arrow", &["numpad7"]),
            k("Right", "axis-x-arrow", &["numpad3"]),
            k("Frame all", "fit-to-screen", &["home"]),
            k("Undo", "undo", &["ctrl", "z"]),
            k("Save", "content-save", &["ctrl", "s"]),
        ],
    },
    AppPreset {
        name: "Browser",
        page: "browser",
        apps: "chrome.exe|msedge.exe|firefox.exe|brave.exe|vivaldi.exe|opera.exe",
        icon: "web",
        color: "#1b4332",
        keys: &[
            k("New tab", "tab-plus", &["ctrl", "t"]),
            k("Close tab", "tab-remove", &["ctrl", "w"]),
            k("Reopen", "tab-unselected", &["ctrl", "shift", "t"]),
            k("Prev tab", "chevron-left", &["ctrl", "shift", "tab"]),
            k("Next tab", "chevron-right", &["ctrl", "tab"]),
            k("Reload", "reload", &["f5"]),
            k("Back", "arrow-left", &["alt", "left"]),
            k("Forward", "arrow-right", &["alt", "right"]),
            k("Address", "web", &["ctrl", "l"]),
            k("Find", "magnify", &["ctrl", "f"]),
            k("Bookmark", "bookmark-plus", &["ctrl", "d"]),
            k("DevTools", "code-tags", &["f12"]),
            k("Private", "incognito", &["ctrl", "shift", "n"]),
        ],
    },
    AppPreset {
        name: "Teams",
        page: "teams",
        apps: "ms-teams.exe|teams.exe",
        icon: "microsoft-teams",
        color: "#4b53bc",
        keys: &[
            k("Mute", "microphone-off", &["ctrl", "shift", "m"]),
            k("Camera", "video", &["ctrl", "shift", "o"]),
            k("Share", "monitor-share", &["ctrl", "shift", "e"]),
            k("Raise hand", "hand-back-left", &["ctrl", "shift", "k"]),
            k("Leave", "phone-hangup", &["ctrl", "shift", "h"]),
            k("Accept", "phone", &["ctrl", "shift", "a"]),
            k("Decline", "phone-off", &["ctrl", "shift", "d"]),
            k("Chat", "chat", &["ctrl", "2"]),
            k("Calendar", "calendar", &["ctrl", "4"]),
            k("Search", "magnify", &["ctrl", "e"]),
            k("Fullscreen", "fullscreen", &["ctrl", "shift", "f"]),
            k("Background", "image-filter-hdr", &["ctrl", "shift", "p"]),
            k("Settings", "cog", &["ctrl", ","]),
        ],
    },
    AppPreset {
        name: "Zoom",
        page: "zoom",
        apps: "zoom.exe",
        icon: "video-account",
        color: "#0b5cff",
        keys: &[
            k("Mute", "microphone-off", &["alt", "a"]),
            k("Camera", "video", &["alt", "v"]),
            k("Share", "monitor-share", &["alt", "s"]),
            k("Raise hand", "hand-back-left", &["alt", "y"]),
            k("Leave", "phone-hangup", &["alt", "q"]),
            k("Chat", "chat", &["alt", "h"]),
            k("Participants", "account-group", &["alt", "u"]),
            k("Record", "record", &["alt", "r"]),
            k("Gallery", "view-grid", &["alt", "f2"]),
            k("Speaker", "account", &["alt", "f1"]),
            k("Fullscreen", "fullscreen", &["alt", "f"]),
            k("Invite", "account-plus", &["alt", "i"]),
            k("Mute all", "volume-mute", &["alt", "m"]),
        ],
    },
    AppPreset {
        name: "OBS Studio",
        page: "obs",
        apps: "obs64.exe|obs32.exe",
        icon: "broadcast",
        color: "#302e31",
        keys: &[
            k("Start stream", "access-point", &["ctrl", "shift", "f9"]),
            k("Stop stream", "access-point-off", &["ctrl", "shift", "f10"]),
            k("Start rec", "record", &["ctrl", "shift", "f11"]),
            k("Stop rec", "stop", &["ctrl", "shift", "f12"]),
            k("Mute mic", "microphone-off", &["ctrl", "shift", "m"]),
            k("Mute desktop", "volume-off", &["ctrl", "shift", "d"]),
            k("Scene 1", "numeric-1-box", &["ctrl", "shift", "1"]),
            k("Scene 2", "numeric-2-box", &["ctrl", "shift", "2"]),
            k("Scene 3", "numeric-3-box", &["ctrl", "shift", "3"]),
            k("Scene 4", "numeric-4-box", &["ctrl", "shift", "4"]),
            k("Transition", "transition", &["ctrl", "shift", "t"]),
            k("Studio mode", "view-split-vertical", &["ctrl", "shift", "s"]),
            k("Replay", "replay", &["ctrl", "shift", "r"]),
        ],
    },
    AppPreset {
        name: "Discord",
        page: "discord",
        apps: "discord.exe",
        icon: "forum",
        color: "#5865f2",
        keys: &[
            k("Mute", "microphone-off", &["ctrl", "shift", "m"]),
            k("Deafen", "headphones-off", &["ctrl", "shift", "d"]),
            k("Answer", "phone", &["ctrl", "enter"]),
            k("Hang up", "phone-hangup", &["escape"]),
            k("Search", "magnify", &["ctrl", "k"]),
            k("Unread", "bell", &["alt", "shift", "down"]),
            k("Prev server", "chevron-up", &["ctrl", "alt", "up"]),
            k("Next server", "chevron-down", &["ctrl", "alt", "down"]),
            k("Mark read", "check-all", &["escape"]),
            k("Pins", "pin", &["ctrl", "p"]),
            k("Mentions", "at", &["ctrl", "i"]),
            k("Emoji", "emoticon", &["ctrl", "e"]),
            k("Settings", "cog", &["ctrl", ","]),
        ],
    },
    AppPreset {
        name: "Spotify",
        page: "spotify-app",
        apps: "spotify.exe",
        icon: "spotify",
        color: "#1db954",
        keys: &[
            k("Play/Pause", "play-pause", &["space"]),
            k("Next", "skip-next", &["ctrl", "right"]),
            k("Previous", "skip-previous", &["ctrl", "left"]),
            k("Vol up", "volume-plus", &["ctrl", "up"]),
            k("Vol down", "volume-minus", &["ctrl", "down"]),
            k("Shuffle", "shuffle", &["ctrl", "s"]),
            k("Repeat", "repeat", &["ctrl", "r"]),
            k("Like", "heart", &["alt", "shift", "b"]),
            k("Search", "magnify", &["ctrl", "l"]),
            k("Home", "home", &["alt", "shift", "h"]),
            k("Queue", "playlist-music", &["alt", "shift", "q"]),
            k("New playlist", "playlist-plus", &["ctrl", "n"]),
            k("Now playing", "music-note", &["alt", "shift", "j"]),
        ],
    },
    AppPreset {
        name: "Excel",
        page: "excel",
        apps: "excel.exe",
        icon: "microsoft-excel",
        color: "#107c41",
        keys: &[
            k("Sum", "sigma", &["alt", "="]),
            k("Bold", "format-bold", &["ctrl", "b"]),
            k("Currency", "currency-usd", &["ctrl", "shift", "4"]),
            k("Percent", "percent", &["ctrl", "shift", "5"]),
            k("Filter", "filter", &["ctrl", "shift", "l"]),
            k("Insert row", "table-row-plus-after", &["ctrl", "+"]),
            k("Delete row", "table-row-remove", &["ctrl", "-"]),
            k("Fill down", "arrow-down-bold", &["ctrl", "d"]),
            k("Fill right", "arrow-right-bold", &["ctrl", "r"]),
            k("Paste values", "content-paste", &["ctrl", "alt", "v"]),
            k("Chart", "chart-bar", &["alt", "f1"]),
            k("Freeze", "snowflake", &["alt", "w", "f", "f"]),
            k("Save", "content-save", &["ctrl", "s"]),
        ],
    },
    AppPreset {
        name: "Explorer",
        page: "explorer",
        apps: "explorer.exe",
        icon: "folder",
        color: "#3a3f44",
        keys: &[
            k("New folder", "folder-plus", &["ctrl", "shift", "n"]),
            k("Rename", "rename-box", &["f2"]),
            k("Copy", "content-copy", &["ctrl", "c"]),
            k("Cut", "content-cut", &["ctrl", "x"]),
            k("Paste", "content-paste", &["ctrl", "v"]),
            k("Delete", "delete", &["delete"]),
            k("Shift del", "delete-forever", &["shift", "delete"]),
            k("Up", "folder-arrow-up", &["alt", "up"]),
            k("Back", "arrow-left", &["alt", "left"]),
            k("Address", "folder-search", &["ctrl", "l"]),
            k("Preview", "eye", &["alt", "p"]),
            k("Properties", "information", &["alt", "enter"]),
            k("Refresh", "refresh", &["f5"]),
        ],
    },
];

/// Running applications with a window, as lowercase exe names (Windows: tasklist /V).
pub fn running_apps() -> Vec<String> {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        let out = std::process::Command::new("tasklist").args(["/V", "/FO", "CSV", "/NH"]).creation_flags(0x0800_0000).output();
        if let Ok(o) = out {
            let text = String::from_utf8_lossy(&o.stdout);
            let mut v: Vec<String> = text
                .lines()
                .filter_map(|l| {
                    let cols: Vec<&str> = l.split("\",\"").collect();
                    if cols.len() < 9 {
                        return None;
                    }
                    let exe = cols[0].trim_start_matches('"').to_lowercase();
                    let title = cols[8].trim_end_matches('"');
                    if title == "N/A" || title.is_empty() || exe == "tasklist.exe" {
                        None
                    } else {
                        Some(exe)
                    }
                })
                .collect();
            v.sort();
            v.dedup();
            return v;
        }
    }
    Vec::new()
}
