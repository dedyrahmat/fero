// Adapted from GPUI 0.2.2's single-line input example. See GPUI-LICENSE-APACHE.
use gpui::{
    App, Bounds, ClipboardItem, Context, CursorStyle, Element, ElementId, ElementInputHandler,
    Entity, EntityInputHandler, FocusHandle, GlobalElementId, KeyBinding, LayoutId, MouseButton,
    Pixels, Point, ShapedLine, Style, TextRun, UTF16Selection, Window, actions, div, fill, point,
    prelude::*, px, relative, size,
};
use std::ops::Range;
use unicode_segmentation::UnicodeSegmentation;

actions!(
    fero_text,
    [
        Backspace,
        Delete,
        Left,
        Right,
        ExtendLeft,
        ExtendRight,
        First,
        Last,
        ExtendFirst,
        ExtendLast,
        SelectAll,
        Copy,
        Cut,
        Paste
    ]
);

pub fn bind(cx: &mut App) {
    let context = Some("FeroText");
    let mac = cfg!(target_os = "macos");
    cx.bind_keys([
        KeyBinding::new("backspace", Backspace, context),
        KeyBinding::new("delete", Delete, context),
        KeyBinding::new("left", Left, context),
        KeyBinding::new("right", Right, context),
        KeyBinding::new("shift-left", ExtendLeft, context),
        KeyBinding::new("shift-right", ExtendRight, context),
        KeyBinding::new("home", First, context),
        KeyBinding::new("end", Last, context),
        KeyBinding::new("shift-home", ExtendFirst, context),
        KeyBinding::new("shift-end", ExtendLast, context),
        KeyBinding::new(if mac { "cmd-left" } else { "ctrl-home" }, First, context),
        KeyBinding::new(if mac { "cmd-right" } else { "ctrl-end" }, Last, context),
        KeyBinding::new(
            if mac {
                "cmd-shift-left"
            } else {
                "ctrl-shift-home"
            },
            ExtendFirst,
            context,
        ),
        KeyBinding::new(
            if mac {
                "cmd-shift-right"
            } else {
                "ctrl-shift-end"
            },
            ExtendLast,
            context,
        ),
        KeyBinding::new(if mac { "cmd-a" } else { "ctrl-a" }, SelectAll, context),
        KeyBinding::new(if mac { "cmd-c" } else { "ctrl-c" }, Copy, context),
        KeyBinding::new(if mac { "cmd-x" } else { "ctrl-x" }, Cut, context),
        KeyBinding::new(if mac { "cmd-v" } else { "ctrl-v" }, Paste, context),
    ]);
}

pub struct TextField {
    pub focus: FocusHandle,
    pub text: String,
    placeholder: &'static str,
    magnet: bool,
    anchor: usize,
    cursor: usize,
    marked: Option<Range<usize>>,
    layout: Option<(ShapedLine, Bounds<Pixels>, Pixels)>,
    selecting: bool,
}

impl TextField {
    pub fn new(placeholder: &'static str, magnet: bool, cx: &mut Context<Self>) -> Self {
        Self {
            focus: cx.focus_handle().tab_stop(true),
            text: String::new(),
            placeholder,
            magnet,
            anchor: 0,
            cursor: 0,
            marked: None,
            layout: None,
            selecting: false,
        }
    }

    pub fn set(&mut self, text: String, cx: &mut Context<Self>) {
        self.anchor = text.len();
        self.cursor = text.len();
        self.text = text;
        self.marked = None;
        self.layout = None;
        cx.notify();
    }

    fn selection(&self) -> Range<usize> {
        self.anchor.min(self.cursor)..self.anchor.max(self.cursor)
    }

    fn move_cursor(&mut self, to: usize, extend: bool, cx: &mut Context<Self>) {
        self.cursor = to;
        if !extend {
            self.anchor = to;
        }
        cx.notify();
    }

    fn boundary(&self, right: bool) -> usize {
        let mut boundaries = self.text.grapheme_indices(true).map(|(i, _)| i);
        if right {
            boundaries
                .find(|&i| i > self.cursor)
                .unwrap_or(self.text.len())
        } else {
            boundaries.rev().find(|&i| i < self.cursor).unwrap_or(0)
        }
    }

    fn arrow(&mut self, right: bool, extend: bool, cx: &mut Context<Self>) {
        let range = self.selection();
        let target = if extend || range.is_empty() {
            self.boundary(right)
        } else if right {
            range.end
        } else {
            range.start
        };
        self.move_cursor(target, extend, cx);
    }

    fn erase(&mut self, right: bool, window: &mut Window, cx: &mut Context<Self>) {
        if self.selection().is_empty() && self.marked.is_none() {
            self.anchor = self.boundary(right);
        }
        self.replace_text_in_range(None, "", window, cx);
    }

    fn copy(&self, cx: &mut Context<Self>) {
        if !self.selection().is_empty() {
            cx.write_to_clipboard(ClipboardItem::new_string(
                self.text[self.selection()].into(),
            ));
        }
    }

    fn byte_offset(text: &str, offset: usize) -> usize {
        let mut units = 0;
        for (index, ch) in text.char_indices() {
            if units >= offset {
                return index;
            }
            units += ch.len_utf16();
        }
        text.len()
    }

    fn utf16_offset(&self, offset: usize) -> usize {
        self.text[..offset].encode_utf16().count()
    }

    fn range_from_utf16(&self, range: Range<usize>) -> Range<usize> {
        Self::byte_offset(&self.text, range.start)..Self::byte_offset(&self.text, range.end)
    }

    fn to_utf16(&self, range: Range<usize>) -> Range<usize> {
        self.utf16_offset(range.start)..self.utf16_offset(range.end)
    }

    fn replace(&mut self, range: Option<Range<usize>>, text: &str) -> Range<usize> {
        let range = range
            .map(|range| self.range_from_utf16(range))
            .or_else(|| self.marked.take())
            .unwrap_or_else(|| self.selection());
        let end = range.start + text.len();
        self.text.replace_range(range.clone(), text);
        self.anchor = end;
        self.cursor = end;
        self.marked = None;
        range.start..end
    }

    fn mouse_index(&self, position: Point<Pixels>) -> usize {
        self.layout.as_ref().map_or(0, |(line, bounds, scroll)| {
            line.closest_index_for_x(position.x - bounds.left() + *scroll)
                .min(self.text.len())
        })
    }
}

impl EntityInputHandler for TextField {
    fn text_for_range(
        &mut self,
        range: Range<usize>,
        actual: &mut Option<Range<usize>>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<String> {
        let range = self.range_from_utf16(range);
        *actual = Some(self.to_utf16(range.clone()));
        Some(self.text[range].into())
    }

    fn selected_text_range(
        &mut self,
        _: bool,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<UTF16Selection> {
        Some(UTF16Selection {
            range: self.to_utf16(self.selection()),
            reversed: self.cursor < self.anchor,
        })
    }

    fn marked_text_range(&self, _: &mut Window, _: &mut Context<Self>) -> Option<Range<usize>> {
        self.marked.clone().map(|range| self.to_utf16(range))
    }

    fn unmark_text(&mut self, _: &mut Window, cx: &mut Context<Self>) {
        self.marked = None;
        cx.notify();
    }

    fn replace_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let text = if self.magnet {
            text.trim_matches(['\n', '\r'])
        } else {
            text
        };
        self.replace(range, &text.replace(['\r', '\n'], " "));
        cx.notify();
    }

    fn replace_and_mark_text_in_range(
        &mut self,
        range: Option<Range<usize>>,
        text: &str,
        selected: Option<Range<usize>>,
        _: &mut Window,
        cx: &mut Context<Self>,
    ) {
        let range = self.replace(range, text);
        if let Some(selected) = selected {
            self.anchor = range.start + Self::byte_offset(text, selected.start);
            self.cursor = range.start + Self::byte_offset(text, selected.end);
        }
        self.marked = (!range.is_empty()).then_some(range);
        cx.notify();
    }

    fn bounds_for_range(
        &mut self,
        range: Range<usize>,
        bounds: Bounds<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<Bounds<Pixels>> {
        let (line, _, scroll) = self.layout.as_ref()?;
        let range = self.range_from_utf16(range);
        Some(Bounds::from_corners(
            point(
                bounds.left() + line.x_for_index(range.start) - *scroll,
                bounds.top(),
            ),
            point(
                bounds.left() + line.x_for_index(range.end) - *scroll,
                bounds.bottom(),
            ),
        ))
    }

    fn character_index_for_point(
        &mut self,
        point: Point<Pixels>,
        _: &mut Window,
        _: &mut Context<Self>,
    ) -> Option<usize> {
        Some(self.utf16_offset(self.mouse_index(point)))
    }
}

struct InputLine(Entity<TextField>);

impl IntoElement for InputLine {
    type Element = Self;
    fn into_element(self) -> Self {
        self
    }
}

impl Element for InputLine {
    type RequestLayoutState = ();
    type PrepaintState = ();

    fn id(&self) -> Option<ElementId> {
        None
    }
    fn source_location(&self) -> Option<&'static core::panic::Location<'static>> {
        None
    }

    fn request_layout(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        window: &mut Window,
        cx: &mut App,
    ) -> (LayoutId, ()) {
        let mut style = Style::default();
        style.size.width = relative(1.).into();
        style.size.height = window.line_height().into();
        (window.request_layout(style, [], cx), ())
    }

    fn prepaint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        _: Bounds<Pixels>,
        _: &mut (),
        _: &mut Window,
        _: &mut App,
    ) {
    }

    fn paint(
        &mut self,
        _: Option<&GlobalElementId>,
        _: Option<&gpui::InspectorElementId>,
        bounds: Bounds<Pixels>,
        _: &mut (),
        _: &mut (),
        window: &mut Window,
        cx: &mut App,
    ) {
        let field = self.0.read(cx);
        let focus = field.focus.clone();
        let style = window.text_style();
        let focused = focus.is_focused(window);
        let empty = field.text.is_empty();
        let text = if empty {
            field.placeholder
        } else {
            &field.text
        };
        let run = TextRun {
            len: text.len(),
            color: if empty {
                style.color.opacity(0.68)
            } else {
                style.color
            },
            font: style.font(),
            background_color: None,
            underline: None,
            strikethrough: None,
        };
        let line = window.text_system().shape_line(
            text.to_owned().into(),
            style.font_size.to_pixels(window.rem_size()),
            &[run],
            None,
        );
        let cursor = line.x_for_index(field.cursor);
        let previous = field.layout.as_ref().map_or(px(0.), |entry| entry.2);
        let scroll = if focused {
            previous
                .min(cursor)
                .max(cursor - bounds.size.width + px(2.))
                .min((line.width - bounds.size.width + px(2.)).max(px(0.)))
        } else {
            px(0.)
        };
        let origin = point(bounds.left() - scroll, bounds.top());
        let selection = field.selection();
        let marked = field.marked.clone();
        window.handle_input(&focus, ElementInputHandler::new(bounds, self.0.clone()), cx);
        if focused && !selection.is_empty() {
            let left = line.x_for_index(selection.start);
            let right = line.x_for_index(selection.end);
            window.paint_quad(fill(
                Bounds::new(
                    point(origin.x + left, origin.y),
                    size(right - left, bounds.size.height),
                ),
                style.color.opacity(0.18),
            ));
        }
        let _ = line.paint(origin, bounds.size.height, window, cx);
        if focused {
            if let Some(marked) = marked {
                let left = line.x_for_index(marked.start);
                let right = line.x_for_index(marked.end);
                window.paint_quad(fill(
                    Bounds::new(
                        point(origin.x + left, bounds.bottom() - px(2.)),
                        size(right - left, px(1.)),
                    ),
                    style.color,
                ));
            }
            if selection.is_empty() {
                window.paint_quad(fill(
                    Bounds::new(
                        point(origin.x + cursor, bounds.top() + px(1.)),
                        size(px(1.), bounds.size.height - px(2.)),
                    ),
                    style.color,
                ));
            }
        }
        self.0
            .update(cx, |field, _| field.layout = Some((line, bounds, scroll)));
    }
}

impl Render for TextField {
    fn render(&mut self, _: &mut Window, cx: &mut Context<Self>) -> impl IntoElement {
        div()
            .id("text")
            .key_context(if self.magnet {
                "FeroText FeroMagnet"
            } else {
                "FeroText"
            })
            .track_focus(&self.focus)
            .w_full()
            .overflow_hidden()
            .cursor(CursorStyle::IBeam)
            .line_height(px(22.))
            .on_action(cx.listener(|field, _: &Backspace, w, cx| field.erase(false, w, cx)))
            .on_action(cx.listener(|field, _: &Delete, w, cx| field.erase(true, w, cx)))
            .on_action(cx.listener(|field, _: &Left, _, cx| field.arrow(false, false, cx)))
            .on_action(cx.listener(|field, _: &Right, _, cx| field.arrow(true, false, cx)))
            .on_action(cx.listener(|field, _: &ExtendLeft, _, cx| field.arrow(false, true, cx)))
            .on_action(cx.listener(|field, _: &ExtendRight, _, cx| field.arrow(true, true, cx)))
            .on_action(cx.listener(|field, _: &First, _, cx| field.move_cursor(0, false, cx)))
            .on_action(
                cx.listener(|field, _: &Last, _, cx| {
                    field.move_cursor(field.text.len(), false, cx)
                }),
            )
            .on_action(cx.listener(|field, _: &ExtendFirst, _, cx| field.move_cursor(0, true, cx)))
            .on_action(cx.listener(|field, _: &ExtendLast, _, cx| {
                field.move_cursor(field.text.len(), true, cx)
            }))
            .on_action(cx.listener(|field, _: &SelectAll, _, cx| {
                field.anchor = 0;
                field.move_cursor(field.text.len(), true, cx);
            }))
            .on_action(cx.listener(|field, _: &Copy, _, cx| field.copy(cx)))
            .on_action(cx.listener(|field, _: &Cut, w, cx| {
                if !field.selection().is_empty() {
                    field.copy(cx);
                    field.replace_text_in_range(None, "", w, cx);
                }
            }))
            .on_action(cx.listener(|field, _: &Paste, w, cx| {
                if let Some(text) = cx.read_from_clipboard().and_then(|item| item.text()) {
                    field.replace_text_in_range(None, &text, w, cx);
                }
            }))
            .on_mouse_down(
                MouseButton::Left,
                cx.listener(|field, event: &gpui::MouseDownEvent, w, cx| {
                    w.focus(&field.focus);
                    field.selecting = true;
                    field.move_cursor(field.mouse_index(event.position), event.modifiers.shift, cx);
                }),
            )
            .on_mouse_move(cx.listener(|field, event: &gpui::MouseMoveEvent, _, cx| {
                if field.selecting && event.pressed_button.is_some() {
                    field.move_cursor(field.mouse_index(event.position), true, cx);
                }
            }))
            .on_mouse_up(
                MouseButton::Left,
                cx.listener(|field, _, _, _| field.selecting = false),
            )
            .on_mouse_up_out(
                MouseButton::Left,
                cx.listener(|field, _, _, _| field.selecting = false),
            )
            .child(InputLine(cx.entity()))
    }
}
