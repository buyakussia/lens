//! Markdown → виджеты GTK. Ответ разбивается на блоки (абзацы, списки, код, таблицы);
//! при потоковом выводе перестраиваются только изменившиеся блоки, поэтому даже длинный
//! ответ печатается плавно.

use gtk::prelude::*;
use pulldown_cmark::{CodeBlockKind, Event, HeadingLevel, Options, Parser, Tag, TagEnd};
use std::cell::RefCell;
use std::rc::Rc;

#[derive(Clone, Debug, PartialEq)]
pub enum Block {
    /// Абзац, заголовок, цитата или список — всё, что рисуется одной подписью с разметкой Pango.
    Text { markup: String, class: &'static str },
    Code { lang: String, code: String },
    Table { rows: Vec<Vec<String>> },
    Rule,
}

pub fn escape(s: &str) -> String {
    let mut o = String::with_capacity(s.len());
    for c in s.chars() {
        match c {
            '&' => o.push_str("&amp;"),
            '<' => o.push_str("&lt;"),
            '>' => o.push_str("&gt;"),
            '"' => o.push_str("&quot;"),
            _ => o.push(c),
        }
    }
    o
}

pub fn parse(text: &str) -> Vec<Block> {
    let mut blocks = vec![];
    let mut cur = String::new();
    let mut class: &'static str = "md-p";
    let mut lists: Vec<Option<u64>> = vec![];
    let mut code: Option<(String, String)> = None;
    let mut table: Option<Vec<Vec<String>>> = None;
    let mut quote = 0;

    let flush = |cur: &mut String, class: &mut &'static str, blocks: &mut Vec<Block>| {
        let t = cur.trim_matches('\n').to_string();
        if !t.trim().is_empty() {
            blocks.push(Block::Text { markup: t, class });
        }
        cur.clear();
        *class = "md-p";
    };

    let opts = Options::ENABLE_TABLES | Options::ENABLE_STRIKETHROUGH;
    for ev in Parser::new_ext(text, opts) {
        if let Some((_, buf)) = code.as_mut() {
            match ev {
                Event::Text(t) => buf.push_str(&t),
                Event::End(TagEnd::CodeBlock) => {
                    let (lang, c) = code.take().unwrap();
                    blocks.push(Block::Code { lang, code: c.trim_end_matches('\n').to_string() });
                }
                _ => {}
            }
            continue;
        }
        match ev {
            Event::Start(Tag::Paragraph) => {
                if lists.is_empty() && quote == 0 {
                    flush(&mut cur, &mut class, &mut blocks);
                } else if !cur.is_empty() && !cur.ends_with('\n') && !cur.ends_with(' ') {
                    cur.push('\n');
                }
            }
            Event::End(TagEnd::Paragraph) => {
                if lists.is_empty() && quote == 0 {
                    flush(&mut cur, &mut class, &mut blocks);
                }
            }
            Event::Start(Tag::Heading { level, .. }) => {
                flush(&mut cur, &mut class, &mut blocks);
                class = match level {
                    HeadingLevel::H1 => "md-h1",
                    HeadingLevel::H2 => "md-h2",
                    _ => "md-h3",
                };
            }
            Event::End(TagEnd::Heading(_)) => flush(&mut cur, &mut class, &mut blocks),
            Event::Start(Tag::BlockQuote(_)) => {
                if quote == 0 {
                    flush(&mut cur, &mut class, &mut blocks);
                }
                quote += 1;
            }
            Event::End(TagEnd::BlockQuote(_)) => {
                quote -= 1;
                if quote == 0 {
                    class = "md-quote";
                    flush(&mut cur, &mut class, &mut blocks);
                }
            }
            Event::Start(Tag::CodeBlock(kind)) => {
                flush(&mut cur, &mut class, &mut blocks);
                let lang = match kind {
                    CodeBlockKind::Fenced(l) => l.split_whitespace().next().unwrap_or("").to_string(),
                    _ => String::new(),
                };
                code = Some((lang, String::new()));
            }
            Event::Start(Tag::List(start)) => {
                if lists.is_empty() {
                    flush(&mut cur, &mut class, &mut blocks);
                }
                lists.push(start);
            }
            Event::End(TagEnd::List(_)) => {
                lists.pop();
                if lists.is_empty() {
                    class = "md-list";
                    flush(&mut cur, &mut class, &mut blocks);
                }
            }
            Event::Start(Tag::Item) => {
                if !cur.is_empty() && !cur.ends_with('\n') {
                    cur.push('\n');
                }
                let depth = lists.len().saturating_sub(1);
                cur.push_str(&"    ".repeat(depth));
                match lists.last_mut() {
                    Some(Some(n)) => {
                        cur.push_str(&format!("{n}. "));
                        *n += 1;
                    }
                    _ => cur.push_str(if depth == 0 { "•  " } else { "◦  " }),
                }
            }
            Event::Start(Tag::Table(_)) => {
                flush(&mut cur, &mut class, &mut blocks);
                table = Some(vec![]);
            }
            Event::Start(Tag::TableHead) | Event::Start(Tag::TableRow) => {
                if let Some(t) = table.as_mut() {
                    t.push(vec![]);
                }
            }
            Event::Start(Tag::TableCell) => cur.clear(),
            Event::End(TagEnd::TableCell) => {
                if let Some(row) = table.as_mut().and_then(|t| t.last_mut()) {
                    row.push(std::mem::take(&mut cur));
                }
            }
            Event::End(TagEnd::Table) => {
                if let Some(rows) = table.take() {
                    blocks.push(Block::Table { rows });
                }
            }
            Event::Start(Tag::Emphasis) => cur.push_str("<i>"),
            Event::End(TagEnd::Emphasis) => cur.push_str("</i>"),
            Event::Start(Tag::Strong) => cur.push_str("<b>"),
            Event::End(TagEnd::Strong) => cur.push_str("</b>"),
            Event::Start(Tag::Strikethrough) => cur.push_str("<s>"),
            Event::End(TagEnd::Strikethrough) => cur.push_str("</s>"),
            Event::Start(Tag::Link { dest_url, .. }) => {
                cur.push_str(&format!("<a href=\"{}\">", escape(&dest_url)))
            }
            Event::End(TagEnd::Link) => cur.push_str("</a>"),
            Event::Code(t) => cur.push_str(&format!(
                "<span font_family=\"monospace\" bgcolor=\"#8080807a\">\u{2009}{}\u{2009}</span>",
                escape(&t)
            )),
            Event::Text(t) => cur.push_str(&escape(&t)),
            Event::SoftBreak => cur.push(' '),
            Event::HardBreak => cur.push('\n'),
            Event::Rule => {
                flush(&mut cur, &mut class, &mut blocks);
                blocks.push(Block::Rule);
            }
            _ => {}
        }
    }
    if let Some((lang, c)) = code {
        blocks.push(Block::Code { lang, code: c.trim_end_matches('\n').to_string() });
    }
    if !lists.is_empty() {
        class = "md-list";
    }
    flush(&mut cur, &mut class, &mut blocks);
    blocks
}

/// Что делать при нажатии «Копировать» у блока кода.
pub type CopyFn = Rc<dyn Fn(&str)>;

fn text_label(markup: &str, class: &str) -> gtk::Label {
    let l = gtk::Label::new(None);
    if !markup.is_empty() {
        l.set_markup(markup);
        // недописанная во время потока разметка может не разобраться — показываем текст как есть
        if l.text().is_empty() {
            l.set_text(markup);
        }
    }
    l.set_wrap(true);
    l.set_wrap_mode(gtk::pango::WrapMode::WordChar);
    l.set_xalign(0.0);
    l.set_selectable(true);
    l.set_can_focus(false);
    l.add_css_class(class);
    l
}

fn build(block: &Block, copy: &CopyFn) -> gtk::Widget {
    match block {
        Block::Text { markup, class } => text_label(markup, class).upcast(),
        Block::Rule => {
            let s = gtk::Separator::new(gtk::Orientation::Horizontal);
            s.add_css_class("md-rule");
            s.upcast()
        }
        Block::Code { lang, code } => {
            let frame = gtk::Box::new(gtk::Orientation::Vertical, 0);
            frame.add_css_class("md-code");
            let head = gtk::Box::new(gtk::Orientation::Horizontal, 6);
            head.add_css_class("md-code-head");
            let l = gtk::Label::new(Some(if lang.is_empty() { "код" } else { lang }));
            l.set_hexpand(true);
            l.set_xalign(0.0);
            head.append(&l);
            let btn = gtk::Button::with_label("Копировать");
            btn.add_css_class("flat");
            btn.add_css_class("md-copy");
            btn.set_can_focus(false);
            let (c, copy) = (code.clone(), copy.clone());
            btn.connect_clicked(move |b| {
                copy(&c);
                b.set_label("Скопировано ✓");
                let b = b.clone();
                gtk::glib::timeout_add_local_once(std::time::Duration::from_millis(1400), move || {
                    b.set_label("Копировать")
                });
            });
            head.append(&btn);
            frame.append(&head);
            let body = gtk::Label::new(Some(code));
            body.set_xalign(0.0);
            body.set_selectable(true);
            body.set_can_focus(false);
            body.add_css_class("md-code-body");
            let sw = gtk::ScrolledWindow::new();
            sw.set_policy(gtk::PolicyType::Automatic, gtk::PolicyType::Never);
            sw.set_propagate_natural_height(true);
            sw.set_child(Some(&body));
            frame.append(&sw);
            frame.upcast()
        }
        Block::Table { rows } => {
            let grid = gtk::Grid::new();
            grid.add_css_class("md-table");
            grid.set_column_spacing(16);
            grid.set_row_spacing(4);
            for (r, row) in rows.iter().enumerate() {
                for (c, cell) in row.iter().enumerate() {
                    let l = text_label(cell, if r == 0 { "md-th" } else { "md-td" });
                    l.set_wrap(true);
                    grid.attach(&l, c as i32, r as i32, 1, 1);
                }
            }
            let sw = gtk::ScrolledWindow::new();
            sw.set_policy(gtk::PolicyType::Automatic, gtk::PolicyType::Never);
            sw.set_propagate_natural_height(true);
            sw.set_child(Some(&grid));
            sw.upcast()
        }
    }
}

/// Отрисованный ответ, который можно обновлять по мере поступления текста.
#[derive(Clone)]
pub struct MarkdownView {
    pub widget: gtk::Box,
    blocks: Rc<RefCell<Vec<(Block, gtk::Widget)>>>,
    copy: CopyFn,
}

impl MarkdownView {
    pub fn new(copy: CopyFn) -> Self {
        let widget = gtk::Box::new(gtk::Orientation::Vertical, 10);
        widget.add_css_class("md");
        MarkdownView { widget, blocks: Rc::new(RefCell::new(vec![])), copy }
    }

    pub fn set_text(&self, text: &str) {
        let new = parse(text);
        let mut old = self.blocks.borrow_mut();
        // общий неизменившийся префикс оставляем как есть
        let mut same = old.iter().zip(new.iter()).take_while(|((a, _), b)| a == *b).count();
        // во время потока обычно просто растёт последний абзац — обновляем подпись на месте
        if old.len() == same + 1 && new.len() > same {
            if let (Block::Text { class: c1, .. }, Block::Text { markup, class: c2 }) = (&old[same].0, &new[same]) {
                if c1 == c2 {
                    if let Some(l) = old[same].1.downcast_ref::<gtk::Label>() {
                        l.set_markup(markup);
                        if l.text().is_empty() {
                            l.set_text(markup);
                        }
                    }
                    old[same].0 = new[same].clone();
                    same += 1;
                }
            }
        }
        for (_, w) in old.drain(same..) {
            self.widget.remove(&w);
        }
        for b in new.into_iter().skip(same) {
            let w = build(&b, &self.copy);
            self.widget.append(&w);
            old.push((b, w));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_blocks() {
        let b = parse("# Заголовок\n\nТекст **жирный** и `код`.\n\n- раз\n- два\n\n```rust\nfn main() {}\n```\n\n| a | b |\n|---|---|\n| 1 | 2 |\n");
        assert!(matches!(&b[0], Block::Text { class: "md-h1", .. }));
        assert!(matches!(&b[1], Block::Text { markup, .. } if markup.contains("<b>жирный</b>")));
        assert!(matches!(&b[2], Block::Text { markup, class: "md-list" } if markup.contains("•  раз\n•  два")));
        assert!(matches!(&b[3], Block::Code { lang, code } if lang == "rust" && code == "fn main() {}"));
        assert!(matches!(&b[4], Block::Table { rows } if rows.len() == 2 && rows[1][1] == "2"));
    }

    #[test]
    fn unfinished_code_block_while_streaming() {
        let b = parse("Вот:\n\n```py\nprint(1)");
        assert!(matches!(&b[1], Block::Code { code, .. } if code == "print(1)"));
    }
}
