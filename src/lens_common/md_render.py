"""Отрисовка Markdown для Lens в виджеты GTK3.

Заголовки, списки (в т.ч. нумерованные и вложенные), цитаты, разделители, таблицы,
жирный/курсив/зачёркнутый, ссылки, inline-код и блоки кода с подсветкой синтаксиса
и кнопкой «Копировать». Цвета — под светлую и тёмную тему.
"""
import re
import html

import gi
gi.require_version("Gtk", "3.0")
from gi.repository import Gtk, GLib

PALETTES = {
    "dark": {
        "text": "#ecebe8", "muted": "#9d9b95", "link": "#a5b4fc",
        "icode_bg": "#3a3a35", "icode_fg": "#c7d2fe",
        "kw": "#c792ea", "str": "#a5d68a", "com": "#7f7d76", "num": "#f5a97f",
        "fn": "#82aaff", "type": "#ffcb6b", "tag": "#f07178", "attr": "#ffcb6b",
    },
    "light": {
        "text": "#1f1e1c", "muted": "#6b6964", "link": "#4338ca",
        "icode_bg": "#ebe8e1", "icode_fg": "#4338ca",
        "kw": "#8839b8", "str": "#3d7a1f", "com": "#8a877f", "num": "#b5531c",
        "fn": "#2a5bc7", "type": "#946200", "tag": "#c02f3e", "attr": "#946200",
    },
}

# ---------------------------------------------------------------- подсветка кода
_KW = {
    "py": "False None True and as assert async await break class continue def del elif else except finally "
          "for from global if import in is lambda nonlocal not or pass raise return try while with yield match case self",
    "js": "break case catch class const continue debugger default delete do else export extends finally for "
          "function if import in instanceof let new return super switch this throw try typeof var void while "
          "with yield async await of null undefined true false static get set interface type enum implements "
          "public private protected readonly as from",
    "c": "auto break case char const continue default do double else enum extern float for goto if inline int "
         "long register return short signed sizeof static struct switch typedef union unsigned void volatile "
         "while bool true false class namespace template typename public private protected virtual override "
         "new delete this nullptr using std include define ifdef ifndef endif",
    "java": "abstract boolean break byte case catch char class continue default do double else enum extends "
            "final finally float for if implements import instanceof int interface long new package private "
            "protected public return short static super switch this throw throws try void while true false null "
            "var val fun when object data sealed override",
    "go": "break case chan const continue default defer else fallthrough for func go goto if import interface "
          "map package range return select struct switch type var nil true false string int error bool",
    "rust": "as async await break const continue crate dyn else enum extern false fn for if impl in let loop "
            "match mod move mut pub ref return self Self static struct super trait true type unsafe use where "
            "while Some None Ok Err",
    "sh": "if then else elif fi for while until do done case esac in function return local export source "
          "echo exit set unset readonly shift cd sudo",
    "sql": "select from where and or not insert into values update set delete create table drop alter add "
           "index primary key foreign references join left right inner outer on as group by order having "
           "limit offset distinct null is in like between union all case when then else end count sum avg",
    "json": "true false null",
    "css": "",
    "html": "",
    "yaml": "true false null yes no on off",
}
_ALIASES = {
    "python": "py", "py3": "py", "python3": "py",
    "javascript": "js", "ts": "js", "typescript": "js", "jsx": "js", "tsx": "js", "mjs": "js", "node": "js",
    "c": "c", "cpp": "c", "c++": "c", "h": "c", "hpp": "c", "cc": "c", "cs": "java", "csharp": "java",
    "java": "java", "kotlin": "java", "kt": "java", "scala": "java", "swift": "java", "dart": "java",
    "php": "js",
    "go": "go", "golang": "go", "rust": "rust", "rs": "rust",
    "sh": "sh", "bash": "sh", "zsh": "sh", "shell": "sh", "console": "sh", "fish": "sh",
    "sql": "sql", "postgres": "sql", "mysql": "sql", "sqlite": "sql",
    "json": "json", "jsonc": "json",
    "css": "css", "scss": "css", "less": "css",
    "html": "html", "xml": "html", "svg": "html", "vue": "html",
    "yaml": "yaml", "yml": "yaml", "toml": "yaml", "ini": "yaml", "conf": "yaml",
}


def _lang(lang):
    lang = (lang or "").strip().lower()
    return _ALIASES.get(lang, lang if lang in _KW else "")


def _span(color, text, bold=False, italic=False):
    attrs = 'foreground="%s"' % color
    if bold:
        attrs += ' weight="bold"'
    if italic:
        attrs += ' style="italic"'
    return "<span %s>%s</span>" % (attrs, html.escape(text, quote=False))


def highlight(code, lang, pal):
    fam = _lang(lang)
    if fam == "html":
        return _highlight_html(code, pal)
    hash_com = fam in ("py", "sh", "yaml")
    parts = []
    if fam in ("js", "c", "java", "go", "rust", "css"):
        parts.append(r"(?P<com>//[^\n]*|/\*.*?\*/)" if fam != "css" else r"(?P<com>/\*.*?\*/)")
    elif fam == "sql":
        parts.append(r"(?P<com>--[^\n]*|/\*.*?\*/)")
    elif hash_com:
        parts.append(r"(?P<com>#[^\n]*)")
    if fam == "py":
        parts.append(r'(?P<str>[rbfuRBFU]{0,2}(?:"""[\s\S]*?(?:"""|$)|\'\'\'[\s\S]*?(?:\'\'\'|$)|"(?:\\.|[^"\\\n])*"?|\'(?:\\.|[^\'\\\n])*\'?))')
    elif fam == "yaml":
        parts.append(r'(?P<str>"(?:\\.|[^"\\\n])*"?|\'[^\'\n]*\'?)')
        parts.append(r"(?P<attr>^[ \t-]*[\w.\-\[\]\"]+(?=\s*[:=]))")
    else:
        parts.append(r'(?P<str>"(?:\\.|[^"\\\n])*"?|\'(?:\\.|[^\'\\\n])*\'?|`(?:\\.|[^`\\])*`?)')
    if fam == "sh":
        parts.append(r"(?P<attr>\$\{?[A-Za-z_][\w]*\}?|\$[0-9@#?*])")
    if fam == "css":
        parts.append(r"(?P<attr>[\w-]+(?=\s*:))")
    parts.append(r"(?P<num>\b(?:0x[0-9a-fA-F]+|\d+(?:\.\d+)?(?:[eE][+-]?\d+)?)\b)")
    parts.append(r"(?P<word>[A-Za-z_][\w]*)(?P<call>\s*\()?")
    rx = re.compile("|".join(parts), re.S | re.M)
    kws = set(_KW.get(fam, "").split())
    if fam == "":
        kws = set()
    ci = fam == "sql"
    out, pos = [], 0
    for m in rx.finditer(code):
        out.append(html.escape(code[pos:m.start()], quote=False))
        pos = m.end()
        gd = m.groupdict()
        if gd.get("word") is not None:
            w = m.group("word")
            call = m.group("call") or ""
            if (w.lower() if ci else w) in kws:
                out.append(_span(pal["kw"], w, bold=False))
            elif call and fam:
                out.append(_span(pal["fn"], w))
            elif fam and w[:1].isupper() and fam in ("js", "c", "java", "go", "rust", "py"):
                out.append(_span(pal["type"], w))
            else:
                out.append(html.escape(w, quote=False))
            out.append(html.escape(call, quote=False))
        elif gd.get("com") is not None:
            out.append(_span(pal["com"], m.group(0), italic=True))
        elif gd.get("str") is not None:
            out.append(_span(pal["str"], m.group(0)))
        elif gd.get("num") is not None:
            out.append(_span(pal["num"], m.group(0)))
        elif gd.get("attr") is not None:
            out.append(_span(pal["attr"], m.group(0)))
        else:
            out.append(html.escape(m.group(0), quote=False))
    out.append(html.escape(code[pos:], quote=False))
    return "".join(out)


def _highlight_html(code, pal):
    rx = re.compile(r"(?P<com><!--.*?(?:-->|$))|(?P<tag></?[\w:-]+)|(?P<attr>[\w:-]+(?==))|"
                    r'(?P<str>"[^"]*"?|\'[^\']*\'?)|(?P<end>/?>)', re.S)
    out, pos = [], 0
    for m in rx.finditer(code):
        out.append(html.escape(code[pos:m.start()], quote=False))
        pos = m.end()
        g = m.lastgroup
        color = {"com": pal["com"], "tag": pal["tag"], "end": pal["tag"],
                 "attr": pal["attr"], "str": pal["str"]}[g]
        out.append(_span(color, m.group(0), italic=(g == "com")))
    out.append(html.escape(code[pos:], quote=False))
    return "".join(out)


# ---------------------------------------------------------------- inline-разметка
def inline(text, pal):
    """Markdown внутри строки → Pango markup."""
    codes = []

    def keep_code(m):
        codes.append(m.group(1))
        return "\x00%d\x00" % (len(codes) - 1)

    s = re.sub(r"`([^`\n]+)`", keep_code, text)
    links = []

    def keep_link(m):
        links.append((m.group(1), m.group(2)))
        return "\x01%d\x01" % (len(links) - 1)

    s = re.sub(r"\[([^\]\n]+)\]\((\S+?)(?:\s+\"[^\"]*\")?\)", keep_link, s)

    def keep_bare(m):
        links.append((m.group(1), m.group(1)))
        return "\x01%d\x01" % (len(links) - 1)

    s = re.sub(r"(?<![\w(\[\x01])(https?://[^\s<>()]+[^\s<>().,;:!?'\"])", keep_bare, s)
    s = html.escape(s, quote=False)
    s = re.sub(r"\*\*\*(.+?)\*\*\*", r"<b><i>\1</i></b>", s)
    s = re.sub(r"\*\*(.+?)\*\*", r"<b>\1</b>", s)
    s = re.sub(r"(?<![\w])__(.+?)__(?![\w])", r"<b>\1</b>", s)
    s = re.sub(r"(?<![\w*])\*(?![\s*])(.+?)(?<![\s*])\*(?![\w*])", r"<i>\1</i>", s)
    s = re.sub(r"(?<![\w])_(?![\s_])(.+?)(?<![\s_])_(?![\w])", r"<i>\1</i>", s)
    s = re.sub(r"~~(.+?)~~", r"<s>\1</s>", s)

    def put_link(m):
        label, url = links[int(m.group(1))]
        label_markup = inline(label, pal) if label != url else html.escape(label, quote=False)
        return '<a href="%s"><span foreground="%s">%s</span></a>' % (
            html.escape(url, quote=True), pal["link"], label_markup)

    s = re.sub(r"\x01(\d+)\x01", put_link, s)

    def put_code(m):
        c = html.escape(codes[int(m.group(1))], quote=False)
        return '<span font_family="monospace" background="%s" foreground="%s"> %s </span>' % (
            pal["icode_bg"], pal["icode_fg"], c)

    return re.sub(r"\x00(\d+)\x00", put_code, s)


# ---------------------------------------------------------------- блоки
_FENCE = re.compile(r"^\s*(```+|~~~+)\s*([\w+#.-]*)")
_LIST = re.compile(r"^(\s*)([-*+]|\d+[.)])\s+(.*)")
_HR = re.compile(r"^\s*([-*_])(\s*\1){2,}\s*$")
_TSEP = re.compile(r"^\s*\|?\s*:?-{2,}:?\s*(\|\s*:?-{2,}:?\s*)*\|?\s*$")


def parse(text):
    """Разбивает Markdown на блоки: (тип, данные)."""
    lines = text.split("\n")
    blocks, i, n = [], 0, len(lines)
    para = []

    def flush():
        if para:
            blocks.append(("p", "\n".join(para)))
            para.clear()

    while i < n:
        ln = lines[i]
        fm = _FENCE.match(ln)
        if fm:
            flush()
            fence, lang = fm.group(1), fm.group(2)
            body = []
            i += 1
            closed = False
            while i < n:
                if lines[i].strip().startswith(fence[0] * 3) and not lines[i].strip().strip(fence[0]):
                    closed = True
                    i += 1
                    break
                body.append(lines[i])
                i += 1
            blocks.append(("code", (lang, "\n".join(body), closed)))
            continue
        if "|" in ln and i + 1 < n and _TSEP.match(lines[i + 1]) and ln.strip():
            flush()
            rows = [ln]
            i += 2
            while i < n and "|" in lines[i] and lines[i].strip():
                rows.append(lines[i])
                i += 1
            blocks.append(("table", [_cells(r) for r in rows]))
            continue
        hm = re.match(r"^(#{1,6})\s+(.*?)\s*#*\s*$", ln)
        if hm:
            flush()
            blocks.append(("h", (len(hm.group(1)), hm.group(2))))
            i += 1
            continue
        if _HR.match(ln):
            flush()
            blocks.append(("hr", None))
            i += 1
            continue
        if _LIST.match(ln):
            flush()
            items = []
            while i < n:
                lm = _LIST.match(lines[i])
                if lm:
                    indent = len(lm.group(1).replace("\t", "    "))
                    items.append([indent, lm.group(2), lm.group(3)])
                elif lines[i].strip() and lines[i].startswith((" ", "\t")) and items:
                    items[-1][2] += "\n" + lines[i].strip()
                elif not lines[i].strip() and i + 1 < n and _LIST.match(lines[i + 1]):
                    pass
                else:
                    break
                i += 1
            blocks.append(("list", items))
            continue
        if ln.lstrip().startswith(">"):
            flush()
            q = []
            while i < n and lines[i].lstrip().startswith(">"):
                q.append(re.sub(r"^\s*>\s?", "", lines[i]))
                i += 1
            blocks.append(("quote", "\n".join(q)))
            continue
        if not ln.strip():
            flush()
            i += 1
            continue
        para.append(ln)
        i += 1
    flush()
    return blocks


def _cells(row):
    r = row.strip()
    if r.startswith("|"):
        r = r[1:]
    if r.endswith("|"):
        r = r[:-1]
    return [c.strip() for c in re.split(r"(?<!\\)\|", r)]


# ---------------------------------------------------------------- виджеты
def _label(markup, cls=None):
    lbl = Gtk.Label()
    lbl.set_line_wrap(True)
    lbl.set_line_wrap_mode(2)  # WORD_CHAR
    lbl.set_selectable(True)
    lbl.set_can_focus(False)
    lbl.set_xalign(0)
    lbl.set_max_width_chars(70)
    try:
        lbl.set_markup(markup)
    except Exception:
        lbl.set_text(re.sub(r"<[^>]+>", "", markup))
    if cls:
        lbl.get_style_context().add_class(cls)
    return lbl


# вызывается перед каждым копированием из приложения — чтобы оно не приняло своё же копирование
# за «только что скопированное пользователем»
on_copy = None


def copy_to_clipboard(text):
    from gi.repository import Gdk
    if on_copy:
        on_copy()
    cb = Gtk.Clipboard.get(Gdk.SELECTION_CLIPBOARD)
    cb.set_text(text, -1)


def _code_block(lang, code, pal):
    box = Gtk.Box(orientation=Gtk.Orientation.VERTICAL)
    box.get_style_context().add_class("codeblock")
    head = Gtk.Box(spacing=6)
    head.get_style_context().add_class("codehead")
    ll = Gtk.Label(label=lang or "код", xalign=0)
    ll.get_style_context().add_class("codelang")
    head.pack_start(ll, True, True, 0)
    btn = Gtk.Button()
    bb = Gtk.Box(spacing=4)
    bb.pack_start(Gtk.Image.new_from_icon_name("edit-copy-symbolic", Gtk.IconSize.MENU), False, False, 0)
    bl = Gtk.Label(label="Копировать")
    bb.pack_start(bl, False, False, 0)
    btn.add(bb)
    btn.get_style_context().add_class("codecopy")
    btn.set_tooltip_text("Скопировать код")

    def on_copy(_b):
        copy_to_clipboard(code)
        bl.set_text("Скопировано ✓")
        GLib.timeout_add(1500, lambda: bl.set_text("Копировать") or False)

    btn.connect("clicked", on_copy)
    head.pack_end(btn, False, False, 0)
    box.pack_start(head, False, False, 0)

    lbl = Gtk.Label()
    lbl.set_selectable(True)
    lbl.set_can_focus(False)
    lbl.set_xalign(0)
    lbl.set_yalign(0)
    lbl.get_style_context().add_class("code")
    try:
        lbl.set_markup(highlight(code, lang, pal))
    except Exception:
        lbl.set_text(code)
    sw = Gtk.ScrolledWindow()
    sw.set_policy(Gtk.PolicyType.AUTOMATIC, Gtk.PolicyType.NEVER)
    sw.set_propagate_natural_height(True)
    sw.add(lbl)
    box.pack_start(sw, False, False, 0)
    return box


def _table(rows, pal):
    grid = Gtk.Grid()
    grid.get_style_context().add_class("mdtable")
    ncols = max(len(r) for r in rows)
    for ri, row in enumerate(rows):
        for ci in range(ncols):
            txt = row[ci] if ci < len(row) else ""
            mk = inline(txt, pal)
            if ri == 0:
                mk = "<b>%s</b>" % mk
            lbl = _label(mk, "cell")
            lbl.set_max_width_chars(max(8, 64 // ncols))
            if ri == 0:
                lbl.get_style_context().add_class("head")
            lbl.set_hexpand(False)
            lbl.set_valign(Gtk.Align.FILL)
            lbl.set_yalign(0)
            grid.attach(lbl, ci, ri, 1, 1)
    # без ScrolledWindow: внутри прокрутки ячейки с переносом строк неверно считают высоту
    grid.set_halign(Gtk.Align.START)
    return grid


_HSIZE = {1: "x-large", 2: "large", 3: "medium", 4: "medium", 5: "medium", 6: "medium"}


def render(container, text, pal):
    """Перерисовывает container (вертикальный Gtk.Box) по тексту Markdown."""
    for c in container.get_children():
        c.destroy()
    for kind, data in parse(text):
        if kind == "p":
            w = _label(inline(data, pal))
        elif kind == "h":
            level, t = data
            w = _label('<span size="%s" weight="bold">%s</span>' % (_HSIZE[level], inline(t, pal)), "mdh")
        elif kind == "hr":
            w = Gtk.Separator()
            w.get_style_context().add_class("mdhr")
        elif kind == "quote":
            w = _label('<span foreground="%s">%s</span>' % (pal["muted"], inline(data, pal)), "quote")
        elif kind == "list":
            base = min(it[0] for it in data)
            lines, counters = [], {}
            for indent, marker, body in data:
                lvl = max(0, (indent - base) // 2)
                lvl = min(lvl, 4)
                if marker[0].isdigit():
                    bullet = marker
                else:
                    bullet = ("•", "◦", "▪", "•", "◦")[lvl]
                body_mk = inline(body, pal).replace("\n", "\n" + " " * (lvl + 1) + " ")
                lines.append("%s%s %s" % (" " * lvl, html.escape(bullet), body_mk))
            w = _label("\n".join(lines))
        elif kind == "code":
            lang, code, _closed = data
            w = _code_block(lang, code, pal)
        elif kind == "table":
            w = _table(data, pal)
        else:
            continue
        container.pack_start(w, False, False, 0)
    container.show_all()
