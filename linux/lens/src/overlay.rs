//! Режим Win+C: экран «замирает», затемняется и подсвечивается по краям; выделенная мышью
//! область сразу становится вложением в чате внизу экрана. Снимок живёт только в памяти.

use crate::chat::ChatView;
use crate::image::{self, Pixels};
use crate::selection::SelectionView;
use adw::prelude::*;
use gtk::graphene::Rect;
use gtk::{gdk, glib};
use std::cell::{Cell, RefCell};
use std::rc::Rc;

pub struct Overlay {
    pub win: gtk::Window,
    sel: SelectionView,
    pixels: RefCell<Option<Pixels>>,
    holder: gtk::Box,
    toolbar: gtk::Box,
    copy_btn: gtk::Button,
    hint: gtk::Label,
    chat: ChatView,
    closing: Cell<bool>,
    on_closed: RefCell<Option<Box<dyn FnOnce()>>>,
}

fn fade(w: &impl IsA<gtk::Widget>, to: f64, ms: u32) {
    let w = w.as_ref().clone();
    let target = adw::CallbackAnimationTarget::new(glib::clone!(
        #[weak]
        w,
        move |v| w.set_opacity(v)
    ));
    let anim = adw::TimedAnimation::new(&w, w.opacity(), to, ms, target);
    anim.set_easing(adw::Easing::EaseOutCubic);
    anim.play();
}

/// Обрезает снимок всех мониторов до первого монитора, если мониторов несколько.
fn texture_for_monitor(tex: gdk::Texture, monitor: &gdk::Monitor) -> gdk::Texture {
    let Some(display) = gdk::Display::default() else { return tex };
    let monitors: Vec<gdk::Monitor> = (0..display.monitors().n_items())
        .filter_map(|i| display.monitors().item(i).and_downcast())
        .collect();
    if monitors.len() <= 1 {
        return tex;
    }
    let (mut x0, mut y0, mut x1, mut y1) = (i32::MAX, i32::MAX, i32::MIN, i32::MIN);
    for m in &monitors {
        let g = m.geometry();
        x0 = x0.min(g.x());
        y0 = y0.min(g.y());
        x1 = x1.max(g.x() + g.width());
        y1 = y1.max(g.y() + g.height());
    }
    let k = tex.width() as f64 / (x1 - x0).max(1) as f64;
    let g = monitor.geometry();
    let px = image::download(&tex);
    let crop = image::crop(
        &px,
        ((g.x() - x0) as f64 * k) as i32,
        ((g.y() - y0) as f64 * k) as i32,
        (g.width() as f64 * k) as i32,
        (g.height() as f64 * k) as i32,
    );
    crop.map(|c| c.upcast()).unwrap_or(tex)
}

impl Overlay {
    pub fn open(app: &adw::Application, tex: gdk::Texture, chat: &ChatView) -> Rc<Overlay> {
        let monitor = linux_common::x11::primary_monitor();
        let tex = match &monitor {
            Some(m) => texture_for_monitor(tex, m),
            None => tex,
        };
        let win = gtk::Window::new();
        win.set_application(Some(app));
        win.set_title(Some("Lens"));
        win.set_decorated(false);
        win.add_css_class("lens-overlay");

        let sel = SelectionView::new(tex);
        let overlay = gtk::Overlay::new();
        overlay.set_child(Some(&sel));

        let hint = gtk::Label::new(Some("Выделите область мышью  ·  Esc — закрыть"));
        hint.add_css_class("lens-hint");
        hint.set_halign(gtk::Align::Center);
        hint.set_valign(gtk::Align::Start);
        hint.set_margin_top(28);
        hint.set_can_target(false);
        hint.set_opacity(0.0);
        overlay.add_overlay(&hint);

        let toolbar = gtk::Box::new(gtk::Orientation::Horizontal, 2);
        toolbar.add_css_class("lens-toolbar");
        toolbar.set_halign(gtk::Align::Start);
        toolbar.set_valign(gtk::Align::Start);
        toolbar.set_visible(false);
        let copy_btn = gtk::Button::new();
        let cb = gtk::Box::new(gtk::Orientation::Horizontal, 6);
        cb.append(&gtk::Image::from_icon_name("edit-copy-symbolic"));
        cb.append(&gtk::Label::new(Some("Копировать")));
        copy_btn.set_child(Some(&cb));
        copy_btn.add_css_class("flat");
        copy_btn.set_tooltip_text(Some("Скопировать выделенное как картинку"));
        copy_btn.set_can_focus(false);
        toolbar.append(&copy_btn);
        overlay.add_overlay(&toolbar);

        let holder = gtk::Box::new(gtk::Orientation::Vertical, 0);
        holder.set_halign(gtk::Align::Center);
        holder.set_valign(gtk::Align::End);
        holder.set_margin_bottom(44);
        holder.set_margin_top(44);
        holder.set_opacity(0.0);
        chat.set_floating(false);
        holder.append(chat.root());
        overlay.add_overlay(&holder);
        win.set_child(Some(&overlay));

        let ov = Rc::new(Overlay {
            win: win.clone(),
            sel: sel.clone(),
            pixels: RefCell::new(None),
            holder: holder.clone(),
            toolbar,
            copy_btn: copy_btn.clone(),
            hint: hint.clone(),
            chat: chat.clone(),
            closing: Cell::new(false),
            on_closed: RefCell::new(None),
        });

        // быстрые действия у выделения: первые три
        for act in chat.0.cfg.borrow().actions.iter().take(3) {
            let b = gtk::Button::with_label(&act.name);
            b.add_css_class("flat");
            b.set_can_focus(false);
            b.set_tooltip_text(Some(&act.prompt));
            let (c, p) = (chat.clone(), act.prompt.clone());
            b.connect_clicked(move |_| c.send(Some(p.clone())));
            ov.toolbar.append(&b);
        }

        let o = Rc::downgrade(&ov);
        sel.connect_selected(move |r| {
            if let Some(o) = o.upgrade() {
                o.on_selected(r);
            }
        });
        let o = Rc::downgrade(&ov);
        sel.connect_drag_start(move || {
            if let Some(o) = o.upgrade() {
                o.toolbar.set_visible(false);
                fade(&o.hint, 0.0, 150);
            }
        });
        let o = Rc::downgrade(&ov);
        sel.connect_cleared(move || {
            if let Some(o) = o.upgrade() {
                o.chat.remove_fragment();
            }
        });
        let o = Rc::downgrade(&ov);
        copy_btn.connect_clicked(move |b| {
            let Some(o) = o.upgrade() else { return };
            if let Some(tex) = o.crop() {
                b.clipboard().set_texture(&tex);
                if let Some(l) = b.child().and_then(|c| c.last_child()).and_downcast::<gtk::Label>() {
                    l.set_text("Скопировано ✓");
                    glib::timeout_add_local_once(std::time::Duration::from_millis(1300), move || l.set_text("Копировать"));
                }
            }
        });

        let keys = gtk::EventControllerKey::new();
        let o = Rc::downgrade(&ov);
        keys.connect_key_pressed(move |_, key, _, _| {
            if key == gdk::Key::Escape {
                if let Some(o) = o.upgrade() {
                    o.close();
                }
                return glib::Propagation::Stop;
            }
            glib::Propagation::Proceed
        });
        win.add_controller(keys);

        match &monitor {
            Some(m) => win.fullscreen_on_monitor(m),
            None => win.fullscreen(),
        }
        win.present();
        chat.focus();

        // карточка чата выезжает снизу, подсказка проявляется
        let h = holder.clone();
        let target = adw::CallbackAnimationTarget::new(move |v| {
            h.set_opacity(v);
            h.set_margin_bottom((24.0 + 20.0 * v) as i32);
        });
        let anim = adw::TimedAnimation::new(&holder, 0.0, 1.0, 320, target);
        anim.set_easing(adw::Easing::EaseOutCubic);
        anim.play();
        fade(&hint, 1.0, 400);
        let hint2 = hint.downgrade();
        glib::timeout_add_local_once(std::time::Duration::from_secs(4), move || {
            if let Some(h) = hint2.upgrade() {
                fade(&h, 0.0, 400);
            }
        });
        ov
    }

    fn crop(&self) -> Option<gdk::Texture> {
        let (x, y, w, h) = self.sel.selection_pixels()?;
        if self.pixels.borrow().is_none() {
            *self.pixels.borrow_mut() = Some(image::download(&self.sel.texture()?));
        }
        let px = self.pixels.borrow();
        image::crop(px.as_ref()?, x, y, w, h).map(|t| t.upcast())
    }

    fn on_selected(&self, r: Rect) {
        let Some(tex) = self.crop() else { return };
        self.chat.add_attachment(image::image_attachment(&tex, "Фрагмент экрана"), Some(tex), true);
        self.chat.focus();

        // карточка не должна закрывать выделение: если оно внизу — переносим чат наверх
        let (w, h) = (self.win.width() as f32, self.win.height() as f32);
        let card_h = self.holder.height().max(140) as f32 + 60.0;
        let card_w = self.holder.width().max(680) as f32;
        let card_x0 = (w - card_w) / 2.0;
        let overlaps_x = r.x() < card_x0 + card_w && r.x() + r.width() > card_x0;
        let low = r.y() + r.height() > h - card_h;
        let high = r.y() < card_h;
        self.holder.set_valign(if overlaps_x && low && !high { gtk::Align::Start } else { gtk::Align::End });

        // панель действий над выделением (или под ним, если сверху нет места)
        self.toolbar.set_visible(true);
        self.toolbar.set_opacity(0.0);
        let (_, nat, _, _) = self.toolbar.measure(gtk::Orientation::Horizontal, -1);
        let tx = r.x().min(w - nat as f32 - 12.0).max(12.0);
        let ty = if r.y() > 64.0 { r.y() - 52.0 } else { (r.y() + r.height() + 12.0).min(h - 56.0) };
        self.toolbar.set_margin_start(tx as i32);
        self.toolbar.set_margin_top(ty as i32);
        fade(&self.toolbar, 1.0, 180);
        let _ = &self.copy_btn;
    }

    pub fn connect_closed(&self, f: impl FnOnce() + 'static) {
        *self.on_closed.borrow_mut() = Some(Box::new(f));
    }

    pub fn is_closing(&self) -> bool {
        self.closing.get()
    }

    /// Плавно закрывает оверлей. Виджет чата остаётся жив (его можно показать в другом окне).
    pub fn close(self: &Rc<Self>) {
        if self.closing.replace(true) {
            return;
        }
        fade(&self.holder, 0.0, 140);
        fade(&self.toolbar, 0.0, 120);
        let o = self.clone();
        self.sel.close(move || o.destroy());
    }

    /// Мгновенно убирает оверлей (например, при откреплении чата).
    pub fn destroy(&self) {
        self.closing.set(true);
        let root = self.chat.root();
        if root.parent().as_ref() == Some(self.holder.upcast_ref()) {
            self.holder.remove(root);
        }
        self.sel.release();
        self.pixels.borrow_mut().take();
        self.win.destroy();
        if let Some(cb) = self.on_closed.borrow_mut().take() {
            cb();
        }
        // снимок экрана занимал десятки мегабайт — возвращаем память системе
        glib::idle_add_local_once(linux_common::trim_memory);
    }
}
