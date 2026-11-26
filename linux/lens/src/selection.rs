//! Полноэкранный «замороженный» снимок экрана, на котором выделяется область.
//!
//! Рисуется через GSK (на видеокарте): снимок, затемнение, переливающаяся подсветка по краям
//! экрана и уголки выделения с мягкой анимацией. Ничего не сохраняется на диск.

use gtk::gdk;
use gtk::glib;
use gtk::graphene::{Point, Rect, Size};
use gtk::gsk::{self, ColorStop, RoundedRect};
use gtk::prelude::*;
use gtk::subclass::prelude::*;
use std::cell::{Cell, RefCell};

type RectCb = Box<dyn Fn(Rect)>;

pub fn ease_out_cubic(t: f64) -> f64 {
    let t = t.clamp(0.0, 1.0);
    1.0 - (1.0 - t).powi(3)
}

mod imp {
    use super::*;

    #[derive(Default)]
    pub struct SelectionView {
        pub texture: RefCell<Option<gdk::Texture>>,
        /// Маленькая маска «светится у краёв»: считается один раз, растягивается видеокартой.
        pub mask: RefCell<Option<gdk::Texture>>,
        pub sel: Cell<Option<Rect>>,
        pub drag_from: Cell<(f64, f64)>,
        pub dragging: Cell<bool>,
        /// Время (мкс) начала открытия / закрытия / появления выделения.
        pub t_open: Cell<i64>,
        pub t_close: Cell<i64>,
        pub t_sel: Cell<i64>,
        pub now: Cell<i64>,
        pub on_selected: RefCell<Option<RectCb>>,
        pub on_cleared: RefCell<Option<Box<dyn Fn()>>>,
        pub on_drag_start: RefCell<Option<Box<dyn Fn()>>>,
        pub on_closed: RefCell<Option<Box<dyn FnOnce()>>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for SelectionView {
        const NAME: &'static str = "LensSelectionView";
        type Type = super::SelectionView;
        type ParentType = gtk::Widget;
    }

    impl ObjectImpl for SelectionView {}

    impl WidgetImpl for SelectionView {
        fn snapshot(&self, snapshot: &gtk::Snapshot) {
            let w = self.obj().width() as f32;
            let h = self.obj().height() as f32;
            if w <= 0.0 || h <= 0.0 {
                return;
            }
            let now = self.now.get();
            let secs = |t: i64| (now - t) as f64 / 1_000_000.0;
            let mut appear = ease_out_cubic(secs(self.t_open.get()) / 0.28);
            if self.t_close.get() > 0 {
                appear *= 1.0 - ease_out_cubic(secs(self.t_close.get()) / 0.16);
            }
            let full = Rect::new(0.0, 0.0, w, h);

            if let Some(tex) = self.texture.borrow().as_ref() {
                snapshot.append_scaled_texture(tex, gsk::ScalingFilter::Linear, &full);
            }

            // затемнение: сильнее вокруг выделения
            let sel = self.sel.get();
            let dim = (if sel.is_some() { 0.52 } else { 0.30 } * appear) as f32;
            let shade = gdk::RGBA::new(0.0, 0.0, 0.0, dim);
            match sel {
                None => snapshot.append_color(&shade, &full),
                Some(r) => {
                    let (x0, y0, x1, y1) = (r.x(), r.y(), r.x() + r.width(), r.y() + r.height());
                    snapshot.append_color(&shade, &Rect::new(0.0, 0.0, w, y0));
                    snapshot.append_color(&shade, &Rect::new(0.0, y1, w, h - y1));
                    snapshot.append_color(&shade, &Rect::new(0.0, y0, x0, y1 - y0));
                    snapshot.append_color(&shade, &Rect::new(x1, y0, w - x1, y1 - y0));
                }
            }

            // переливающаяся подсветка по краям: радужный градиент под маской,
            // которая видна только у краёв экрана
            let t = secs(self.t_open.get());
            let glow = (0.75 * appear) as f32;
            let breathe = 0.82 + 0.18 * (t * 1.7).sin() as f32;
            snapshot.push_mask(gsk::MaskMode::Alpha);
            if let Some(mask) = self.mask.borrow().as_ref() {
                snapshot.append_scaled_texture(mask, gsk::ScalingFilter::Linear, &full);
            }
            snapshot.pop();
            let c = |r: f32, g: f32, b: f32| gdk::RGBA::new(r, g, b, glow * breathe);
            let stops = [
                ColorStop::new(0.00, c(0.98, 0.33, 0.30)),
                ColorStop::new(0.18, c(1.00, 0.76, 0.20)),
                ColorStop::new(0.36, c(0.30, 0.85, 0.45)),
                ColorStop::new(0.55, c(0.26, 0.55, 1.00)),
                ColorStop::new(0.75, c(0.66, 0.40, 1.00)),
                ColorStop::new(1.00, c(0.98, 0.33, 0.30)),
            ];
            // цвета медленно «текут» по диагонали; повторяющийся градиент бесшовен (первый цвет = последнему)
            let period = (w + h) * 0.75;
            let phase = ((t * 0.08) as f32 % 1.0) * period;
            let (dx, dy) = (0.8f32, 0.6f32);
            let start = Point::new(-phase * dx, -phase * dy);
            let end = Point::new(start.x() + period * dx, start.y() + period * dy);
            snapshot.append_repeating_linear_gradient(&full, &start, &end, &stops);
            snapshot.pop();

            if let Some(r) = sel {
                draw_selection(snapshot, r, secs(self.t_sel.get()), self.dragging.get(), appear as f32);
            }
        }
    }

    fn draw_selection(s: &gtk::Snapshot, r: Rect, since: f64, dragging: bool, appear: f32) {
        // лёгкое «вздрагивание» уголков после отпускания мыши
        let pop = if dragging { 0.0 } else { (1.0 - ease_out_cubic(since / 0.35)) as f32 };
        let off = 3.0 + 7.0 * pop;
        let white = gdk::RGBA::new(1.0, 1.0, 1.0, appear);
        let soft = gdk::RGBA::new(1.0, 1.0, 1.0, 0.18 * appear);
        let radius = 14.0f32.min(r.width() / 3.0).min(r.height() / 3.0);
        let outline = RoundedRect::from_rect(r, radius);
        s.append_outset_shadow(&outline, &gdk::RGBA::new(1.0, 1.0, 1.0, 0.10 * appear), 0.0, 0.0, 2.0, 18.0);
        s.append_border(&outline, &[1.0; 4], &[soft; 4]);

        let len = 26.0f32.min(r.width() / 2.0).min(r.height() / 2.0).max(8.0);
        let th = 4.5f32;
        let (x0, y0) = (r.x() - off, r.y() - off);
        let (x1, y1) = (r.x() + r.width() + off, r.y() + r.height() + off);
        let bar = |x: f32, y: f32, bw: f32, bh: f32| {
            let rr = RoundedRect::from_rect(Rect::new(x, y, bw, bh), th / 2.0);
            s.push_rounded_clip(&rr);
            s.append_color(&white, &Rect::new(x, y, bw, bh));
            s.pop();
        };
        // четыре уголка: горизонтальная и вертикальная планки
        bar(x0, y0, len, th);
        bar(x0, y0, th, len);
        bar(x1 - len, y0, len, th);
        bar(x1 - th, y0, th, len);
        bar(x0, y1 - th, len, th);
        bar(x0, y1 - len, th, len);
        bar(x1 - len, y1 - th, len, th);
        bar(x1 - th, y1 - len, th, len);
        let _ = Size::new(0.0, 0.0);
    }
}

/// Маска подсветки: непрозрачна у краёв и плавно исчезает к центру (~6% ширины экрана).
fn edge_mask(w: i32, h: i32) -> gdk::Texture {
    let mw = 320usize;
    let mh = ((mw as f64 * h.max(1) as f64 / w.max(1) as f64).round() as usize).clamp(60, 640);
    let falloff = mw as f64 * 0.05;
    let mut data = vec![0u8; mw * mh * 4];
    for y in 0..mh {
        for x in 0..mw {
            // мягкие углы без «шва»: свечение от вертикальных и горизонтальных краёв складывается
            let edge = |d: f64| {
                let a = (1.0 - d / falloff).clamp(0.0, 1.0);
                a * a * (3.0 - 2.0 * a)
            };
            let ax = edge((x as f64 + 0.5).min(mw as f64 - x as f64 - 0.5));
            let ay = edge((y as f64 + 0.5).min(mh as f64 - y as f64 - 0.5));
            let a = 1.0 - (1.0 - ax) * (1.0 - ay);
            let i = (y * mw + x) * 4;
            data[i..i + 4].copy_from_slice(&[255, 255, 255, (a * 255.0) as u8]);
        }
    }
    gdk::MemoryTexture::new(mw as i32, mh as i32, gdk::MemoryFormat::R8g8b8a8, &glib::Bytes::from_owned(data), mw * 4)
        .upcast()
}

glib::wrapper! {
    pub struct SelectionView(ObjectSubclass<imp::SelectionView>) @extends gtk::Widget,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl SelectionView {
    pub fn new(texture: gdk::Texture) -> Self {
        let v: Self = glib::Object::new();
        v.set_hexpand(true);
        v.set_vexpand(true);
        v.set_cursor_from_name(Some("crosshair"));
        *v.imp().mask.borrow_mut() = Some(edge_mask(texture.width(), texture.height()));
        *v.imp().texture.borrow_mut() = Some(texture);

        let drag = gtk::GestureDrag::new();
        drag.set_button(gdk::BUTTON_PRIMARY);
        let this = v.downgrade();
        drag.connect_drag_begin(move |_, x, y| {
            let Some(v) = this.upgrade() else { return };
            let i = v.imp();
            i.drag_from.set((x, y));
            i.dragging.set(true);
            i.sel.set(None);
            if let Some(cb) = i.on_drag_start.borrow().as_ref() {
                cb();
            };
        });
        let this = v.downgrade();
        drag.connect_drag_update(move |_, dx, dy| {
            let Some(v) = this.upgrade() else { return };
            let i = v.imp();
            let (x, y) = i.drag_from.get();
            let (x1, y1) = (x + dx, y + dy);
            let r = Rect::new(x.min(x1) as f32, y.min(y1) as f32, dx.abs() as f32, dy.abs() as f32);
            i.sel.set(if r.width() > 2.0 || r.height() > 2.0 { Some(r) } else { None });
        });
        let this = v.downgrade();
        drag.connect_drag_end(move |_, _, _| {
            let Some(v) = this.upgrade() else { return };
            let i = v.imp();
            i.dragging.set(false);
            i.t_sel.set(i.now.get());
            match i.sel.get() {
                Some(r) if r.width() >= 8.0 && r.height() >= 8.0 => {
                    if let Some(cb) = i.on_selected.borrow().as_ref() {
                        cb(r);
                    }
                }
                _ => {
                    i.sel.set(None);
                    if let Some(cb) = i.on_cleared.borrow().as_ref() {
                        cb();
                    }
                }
            }
        });
        v.add_controller(drag);

        v.add_tick_callback(|w, clock| {
            let i = w.imp();
            let t = clock.frame_time();
            if i.t_open.get() == 0 {
                i.t_open.set(t);
            }
            i.now.set(t);
            w.queue_draw();
            if i.t_close.get() > 0 && t - i.t_close.get() > 170_000 {
                if let Some(cb) = i.on_closed.borrow_mut().take() {
                    cb();
                }
                return glib::ControlFlow::Break;
            }
            glib::ControlFlow::Continue
        });
        v
    }

    pub fn texture(&self) -> Option<gdk::Texture> {
        self.imp().texture.borrow().clone()
    }

    pub fn selection(&self) -> Option<Rect> {
        self.imp().sel.get()
    }

    /// Выделение в пикселях снимка: (x, y, ширина, высота).
    pub fn selection_pixels(&self) -> Option<(i32, i32, i32, i32)> {
        let r = self.selection()?;
        let tex = self.texture()?;
        let kx = tex.width() as f32 / self.width().max(1) as f32;
        let ky = tex.height() as f32 / self.height().max(1) as f32;
        Some((
            (r.x() * kx).round() as i32,
            (r.y() * ky).round() as i32,
            (r.width() * kx).round() as i32,
            (r.height() * ky).round() as i32,
        ))
    }

    pub fn set_selection(&self, r: Option<Rect>) {
        self.imp().sel.set(r);
        self.imp().t_sel.set(self.imp().now.get());
    }

    pub fn connect_selected(&self, f: impl Fn(Rect) + 'static) {
        *self.imp().on_selected.borrow_mut() = Some(Box::new(f));
    }
    pub fn connect_cleared(&self, f: impl Fn() + 'static) {
        *self.imp().on_cleared.borrow_mut() = Some(Box::new(f));
    }
    pub fn connect_drag_start(&self, f: impl Fn() + 'static) {
        *self.imp().on_drag_start.borrow_mut() = Some(Box::new(f));
    }

    /// Плавно гасит затемнение и подсветку, затем вызывает `done`.
    pub fn close(&self, done: impl FnOnce() + 'static) {
        let i = self.imp();
        if i.t_close.get() > 0 {
            return;
        }
        i.t_close.set(i.now.get().max(1));
        *i.on_closed.borrow_mut() = Some(Box::new(done));
    }

    /// Освобождает снимок (он занимает десятки мегабайт).
    pub fn release(&self) {
        self.imp().texture.borrow_mut().take();
        self.imp().mask.borrow_mut().take();
    }
}
