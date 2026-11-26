pub const CSS: &str = r#"
window.clip-window { background: transparent; box-shadow: none; }
.clip-card {
  margin: 24px;
  background-color: @popover_bg_color;
  color: @popover_fg_color;
  border-radius: 20px;
  box-shadow: 0 6px 20px rgba(0,0,0,0.30), 0 1px 4px rgba(0,0,0,0.18), inset 0 0 0 1px alpha(@window_fg_color, 0.09);
}
.clip-head { padding: 12px 10px 4px 18px; }
.clip-title { font-weight: 700; font-size: 15px; }
.clip-search { margin: 6px 12px 4px 12px; border-radius: 10px; }
.clip-list { background: transparent; padding: 4px 8px 8px 8px; }
.clip-list > row {
  border-radius: 12px; margin: 3px 0; padding: 0;
  background: alpha(@window_fg_color, 0.045);
  transition: background 140ms ease;
}
.clip-list > row:hover { background: alpha(@window_fg_color, 0.08); }
.clip-list > row:selected, .clip-list > row:focus { background: alpha(@accent_bg_color, 0.20); color: inherit; outline: none; }
.clip-item { padding: 10px 6px 10px 14px; }
.clip-text { font-size: 14px; }
.clip-meta { font-size: 11.5px; opacity: 0.6; }
.clip-item picture { border-radius: 8px; }
.clip-item button { min-width: 28px; min-height: 28px; padding: 0; border-radius: 8px; }
.clip-pinned { color: @accent_color; opacity: 1; }
.clip-empty { opacity: 0.55; margin: 40px 20px; }
.clip-hint { font-size: 11.5px; opacity: 0.5; padding: 0 16px 12px 16px; }
"#;
