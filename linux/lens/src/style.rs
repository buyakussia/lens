//! Оформление. Цвета берутся из libadwaita, поэтому светлая и тёмная темы GNOME
//! подхватываются автоматически; акцент — тёплый оранжевый.

pub const ACCENT: (f32, f32, f32) = (0.851, 0.467, 0.341); // #D97757

pub const CSS: &str = r#"
window.lens-overlay { background: black; }
window.lens-transparent { background: transparent; box-shadow: none; }

/* карточка чата — как быстрая панель Claude */
.lens-card {
  background-color: @popover_bg_color;
  color: @popover_fg_color;
  border-radius: 24px;
  box-shadow: 0 18px 48px rgba(0,0,0,0.42), 0 2px 8px rgba(0,0,0,0.22), inset 0 0 0 1px alpha(@window_fg_color, 0.09);
}
.lens-card.floating, .lens-card.lens-quiet {
  margin: 24px;
  box-shadow: 0 6px 20px rgba(0,0,0,0.30), 0 1px 4px rgba(0,0,0,0.18), inset 0 0 0 1px alpha(@window_fg_color, 0.09);
}
.lens-head { padding: 8px 10px 0 16px; }
.lens-title { font-weight: 600; opacity: 0.85; }
.lens-input-row { padding: 10px 10px 10px 12px; }
.lens-entry, .lens-entry > text { background: transparent; color: inherit; font-size: 15.5px; caret-color: #D97757; }
.lens-entry { min-height: 24px; }
.lens-placeholder { opacity: 0.45; font-size: 15.5px; }

button.lens-round {
  min-width: 34px; min-height: 34px; padding: 0; border-radius: 12px;
  transition: background 160ms ease, opacity 160ms ease, transform 160ms ease;
}
button.lens-send { background: #D97757; color: white; }
button.lens-send:hover { background: #e0866a; }
button.lens-send:active { background: #c4664a; }
button.lens-send:disabled { background: alpha(#D97757, 0.35); color: alpha(white, 0.8); }
button.lens-send.stop { background: alpha(@window_fg_color, 0.14); color: @window_fg_color; }

menubutton.lens-model > button {
  border-radius: 10px; padding: 4px 10px; min-height: 26px; font-size: 13.5px;
  background: transparent; opacity: 0.8;
}
menubutton.lens-model > button:hover { background: alpha(@window_fg_color, 0.08); opacity: 1; }

/* вложения */
.lens-chip {
  background: alpha(@window_fg_color, 0.07); border-radius: 12px; padding: 4px 6px 4px 4px;
  transition: background 150ms ease;
}
.lens-chip:hover { background: alpha(@window_fg_color, 0.11); }
.lens-chip-thumb { border-radius: 8px; }
.lens-chip label { font-size: 13px; }
.lens-chip-icon { min-width: 40px; min-height: 40px; border-radius: 8px; background: alpha(@window_fg_color, 0.08); }
button.lens-chip-x { min-width: 22px; min-height: 22px; padding: 0; border-radius: 99px; }

.lens-actions button {
  border-radius: 99px; padding: 4px 12px; min-height: 26px; font-size: 13px;
  background: alpha(@window_fg_color, 0.07);
}
.lens-actions button:hover { background: alpha(@window_fg_color, 0.13); }

/* сообщения */
.lens-messages { padding: 6px 18px 4px 18px; }
.lens-user {
  background: alpha(@window_fg_color, 0.075); border-radius: 18px 18px 6px 18px;
  padding: 9px 14px;
}
.lens-user label { font-size: 14.5px; }
.lens-msg-atts label { font-size: 12.5px; opacity: 0.7; }
.lens-error { color: @error_color; }
.lens-status { font-size: 13px; opacity: 0.65; }

.md { font-size: 14.5px; }
.md-p, .md-list, .md-td { line-height: 1.45; }
.md-h1 { font-size: 19px; font-weight: 700; }
.md-h2 { font-size: 17px; font-weight: 700; }
.md-h3 { font-size: 15px; font-weight: 700; }
.md-quote { border-left: 3px solid alpha(@window_fg_color, 0.25); padding-left: 10px; opacity: 0.85; }
.md-th { font-weight: 700; }
.md-code { background: alpha(@window_fg_color, 0.06); border-radius: 12px; }
.md-code-head { padding: 2px 4px 0 12px; font-size: 12px; }
.md-code-head > label { opacity: 0.6; }
button.md-copy { font-size: 12px; padding: 2px 8px; min-height: 22px; }
.md-code-body { font-family: monospace; font-size: 13px; padding: 4px 12px 12px 12px; }

/* панель действий у выделения */
.lens-toolbar {
  background-color: @popover_bg_color; border-radius: 14px; padding: 4px;
  box-shadow: 0 8px 24px rgba(0,0,0,0.4), inset 0 0 0 1px alpha(@window_fg_color, 0.09);
}
.lens-toolbar button { border-radius: 10px; padding: 6px 10px; font-size: 13.5px; }

.lens-hint {
  background: rgba(20,20,20,0.72); color: white; border-radius: 99px; padding: 8px 16px; font-size: 13.5px;
}

/* тихий режим */
.lens-quiet { border-radius: 18px; }
.lens-quiet .md { font-size: 14px; }
.lens-quiet-head { padding: 8px 6px 2px 14px; }
.lens-quiet-head button { min-width: 26px; min-height: 26px; padding: 0; border-radius: 8px; }
.lens-shimmer { font-size: 13.5px; font-weight: 600; }

/* история */
.lens-history row { border-radius: 10px; padding: 6px 8px; }
.lens-history .dim { font-size: 12px; opacity: 0.6; }

/* настройка сочетания клавиш */
.lens-key-capture { font-size: 18px; font-weight: 700; padding: 18px; }
"#;
