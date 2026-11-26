# Lens & Clipboard History

Два небольших нативных приложения для Linux (GNOME, Wayland):

- **Lens** — выделите любую часть экрана и сразу спросите о ней [Claude Code](https://claude.com/claude-code). Как Google Lens, только с ним можно разговаривать.
- **Clipboard History** — история буфера обмена на <kbd>Super</kbd>+<kbd>V</kbd>, как в Windows, но вся история сохраняется после перезагрузки, а лимиты больше.

> Неофициальный проект, не связан с Anthropic и не одобрен ею. «Claude» — товарный знак Anthropic, PBC.
> Lens сам не обращается ни к какому API: он запускает установленную у вас команду `claude`, в которую вы уже вошли.

*English summary below.*

## Lens

| Клавиши | Что происходит |
|---|---|
| <kbd>Super</kbd>+<kbd>C</kbd> | Открыть или закрыть чат. Чтобы спросить о части экрана, нажмите в чате кнопку ⌜⌟ «Выделить область» (или <kbd>Ctrl</kbd>+<kbd>S</kbd>): экран «замирает», края подсвечиваются, выделенная мышью область возвращается в чат вложением. |
| <kbd>Super</kbd>+<kbd>Alt</kbd>+<kbd>C</kbd> | **Тихий режим**: Lens смотрит на весь экран и коротко отвечает в маленьком окне в правом нижнем углу. Окно не забирает фокус, так что видео продолжает играть. |
| <kbd>Super</kbd>+<kbd>Shift</kbd>+<kbd>C</kbd> | Спросить про текст, выделенный мышью. |

В чате:

- **Скрепка** <kbd>+</kbd>, перетаскивание или <kbd>Ctrl</kbd>+<kbd>V</kbd> прикрепляют картинки, PDF, код и текстовые файлы.
- **Модель** (Авто / Opus / Sonnet / Haiku) выбирается прямо в строке ввода.
- Ответ печатается потоком, с Markdown, таблицами и блоками кода с кнопкой «Копировать».
- **Быстрые действия** («Перевести», «Объяснить», «Распознать текст»…) появляются над полем ввода, когда что-то прикреплено; <kbd>Alt</kbd>+<kbd>1…9</kbd>. Их можно менять в настройках.
- Если хочется сразу выделять область по сочетанию клавиш, назначьте его в настройках («Сразу выделить область и спросить», команда `lens --select`).
- <kbd>Ctrl</kbd>+<kbd>H</kbd> — история чатов, <kbd>Ctrl</kbd>+<kbd>N</kbd> — новый чат, <kbd>Esc</kbd> — закрыть.

**Снимки экрана не сохраняются на диск.** Выделенная область передаётся Claude прямо в сообщении и живёт только в памяти, пока открыт чат.

## Clipboard History

Работает как Win+V в Windows:

- Записывает обычный текст, форматированный текст (HTML) и картинки. Файлы и папки не записываются, как и то, что менеджеры паролей помечают секретным.
- По умолчанию хранит 200 записей (у Windows 25) и картинки до 16 МБ (у Windows 4 МБ). Оба лимита настраиваются.
- **Вся история переживает перезагрузку**, а не только закреплённые записи. Закреплённые не удаляются ни при очистке, ни при переполнении.
- <kbd>Enter</kbd> вставляет в окно, где вы были; <kbd>Shift</kbd>+<kbd>Enter</kbd> вставляет в терминал; <kbd>Ctrl</kbd>+<kbd>Enter</kbd> или меню «⋯» → «Вставить без форматирования» вставляет текст без шрифтов и цветов источника.
- <kbd>Ctrl</kbd>+<kbd>P</kbd> закрепляет, <kbd>Delete</kbd> удаляет, поиск — просто начните печатать.
- «⋯» → «Спросить Lens» отправляет запись в Lens.

## Установка

Нужны GTK 4.14+ и libadwaita 1.5+ (есть в любом свежем дистрибутиве с GNOME), а для Lens — [Claude Code](https://claude.com/claude-code), в который вы уже вошли.

**Готовая сборка:** скачайте `lens-linux-x86_64.tar.gz` со страницы Releases, распакуйте и запустите `./install.sh`.

**Из исходников** (нужен [Rust](https://rustup.rs)):

```bash
sudo apt install libgtk-4-dev libadwaita-1-dev   # Fedora: gtk4-devel libadwaita-devel; Arch: gtk4 libadwaita
git clone <этот репозиторий> && cd lens
./install.sh            # оба приложения; или: ./install.sh lens | ./install.sh clipboard
```

Всё ставится в `~/.local`, оба приложения запускаются в фоне при входе в систему. `./uninstall.sh` удаляет их (с `--purge` — вместе с историей и настройками). Прежнюю версию на Python установщик заменяет сам, история буфера обмена переносится.

### Один раз при первом запуске

- **Снимки экрана.** При первом <kbd>Super</kbd>+<kbd>C</kbd> Lens попросит разрешить снимки экрана без подтверждения, а GNOME спросит один раз. После этого окно скриншота больше не появляется.
- **Автовставка** в Clipboard History нажимает <kbd>Ctrl</kbd>+<kbd>V</kbd> за вас через портал Remote Desktop: GNOME один раз спросит разрешение и во время вставки ненадолго показывает значок удалённого управления. Её можно отключить в настройках, тогда запись только копируется.
- Горячие клавиши регистрируются как пользовательские сочетания GNOME; поменять их можно в настройках каждого приложения.

## Как устроено

```
crates/lens-core    ядро Lens без GUI: запуск Claude Code, настройки, история чатов
crates/clip-core    ядро истории буфера без GUI: хранилище, лимиты, закрепление
linux/common        интеграция с GNOME: порталы, горячие клавиши, автозапуск, окна поверх остальных
linux/lens          Lens для Linux (GTK 4 + libadwaita)
linux/clipboard     Clipboard History для Linux (GTK 4 + libadwaita)
macos/, windows/    план нативных версий (пока не начаты)
```

Написано на Rust; интерфейс — нативный GTK 4 и libadwaita, поэтому приложения сами следуют светлой и тёмной теме GNOME. Оверлей выделения и анимации рисуются видеокартой. Большие объекты (снимок экрана, окна) создаются только когда нужны и сразу освобождаются; в фоне приложения просто ждут горячую клавишу.

Окна работают через XWayland: только так GNOME позволяет обычному приложению держать окно поверх остальных, показывать окно без фокуса и следить за буфером обмена.

## Конфиденциальность

Всё хранится у вас: история чатов — в `~/.local/share/lens`, история буфера — в `~/.local/share/clipboard-history`. Claude (через Claude Code) получает только то, что вы сами отправили.

## Лицензия

MIT — см. [LICENSE](LICENSE).

---

## English summary

**Lens** — <kbd>Super</kbd>+<kbd>C</kbd> opens a chat bar (attach files, pick the model inline); its "select area" button (<kbd>Ctrl</kbd>+<kbd>S</kbd>) freezes the screen with a glowing edge, and the area you drag over comes back into the chat as an attachment. <kbd>Super</kbd>+<kbd>Alt</kbd>+<kbd>C</kbd> is a *quiet mode*: Lens looks at the whole screen and answers in a small corner popup that never takes focus. Screenshots are never written to disk. Answers come from the `claude` CLI you have installed and signed in to.

**Clipboard History** — <kbd>Super</kbd>+<kbd>V</kbd> like on Windows: text, rich text (HTML) and images, pinning, search, paste as plain text. Unlike Windows, the whole history survives reboots and limits are higher (200 items, images up to 16 MB, both configurable).

Native Rust + GTK 4/libadwaita apps for GNOME. Install: `./install.sh` (needs GTK 4.14+, libadwaita 1.5+, and Rust to build from source). Platform-independent cores live in `crates/` so native macOS and Windows front ends can be added later — see `macos/README.md` and `windows/README.md`.
