use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};

use helix_core::graphemes::prev_grapheme_boundary;
use helix_core::{textobject, Range, Rope, Selection, Transaction};
//use crate::commands::{collapse_selection, extend_to_line_bounds, select_mode, Context};
use crate::commands::*;
use helix_view::document::Mode;

#[derive(Default)]
pub struct AtomicState {
    flag: AtomicBool,
    counter: AtomicUsize,
    version: AtomicU64,
}

/// Represents special cases where the target text of Evil operators
/// cannot be determined by a simple motion alone.
#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum EvilOpsCase {
    /// Applies the operator to complete lines, as in `dd`, `yy`, or Visual Line mode.
    CompleteLines,

    /// Applies the operator to the motion, excluding the anchor,
    /// and limiting the operator effect to the current line (e.g. `dw`, `dW`)
    NextWord,
}

pub static VIM_STATE: AtomicState = AtomicState::new();

impl AtomicState {
    pub const fn new() -> Self {
        Self {
            flag: AtomicBool::new(false),
            counter: AtomicUsize::new(0),
            version: AtomicU64::new(0),
        }
    }

    // Example methods
    pub fn visual_line(&self) {
        self.flag.store(true, Ordering::Relaxed);
    }

    // Example methods
    pub fn exit_visual_line(&self) {
        self.flag.store(false, Ordering::Relaxed);
    }

    pub fn is_visual_line(&self) -> bool {
        self.flag.load(Ordering::Acquire)
    }

    pub fn get_counter(&self) -> usize {
        self.counter.load(Ordering::Relaxed)
    }

    pub fn increment_counter(&self) {
        self.counter.fetch_add(1, Ordering::Relaxed);
    }

    pub fn get_version(&self) -> u64 {
        self.version.load(Ordering::Acquire)
    }
}

pub struct VimOps;

impl VimOps {
    pub fn hook_after_each_command(cx: &mut Context) {
        if cx.editor.mode != Mode::Select {
            collapse_selection(cx);
        } else {
            // what
            if VIM_STATE.is_visual_line() {
                extend_to_line_bounds(cx);
            }
        }
    }
}

macro_rules! wrap_hooks {
    // with both before and after
    ($wrapper:ident, $func:path, before = $before:expr, after = $after:expr) => {
        pub fn $wrapper(cx: &mut Context) {
            $before(cx);
            $func(cx);
            $after(cx);
        }
    };

    // with only before
    ($wrapper:ident, $func:path, before = $before:expr) => {
        pub fn $wrapper(cx: &mut Context) {
            $before(cx);
            $func(cx);
        }
    };

    // with only after
    ($wrapper:ident, $func:path, after = $after:expr) => {
        pub fn $wrapper(cx: &mut Context) {
            $func(cx);
            $after(cx);
        }
    };
}

macro_rules! wrap_many_with_hooks {
    (
        [ $( ( $wrapper:ident, $func:path ) ),+ $(,)? ],
        before = $before:expr,
        after = $after:expr
    ) => {
        $(
            wrap_hooks!($wrapper, $func, before = $before, after = $after);
        )+
    };

    (
        [ $( ( $wrapper:ident, $func:path ) ),+ $(,)? ],
        before = $before:expr
    ) => {
        $(
            wrap_hooks!($wrapper, $func, before = $before);
        )+
    };

    (
        [ $( ( $wrapper:ident, $func:path ) ),+ $(,)? ],
        after = $after:expr
    ) => {
        $(
            wrap_hooks!($wrapper, $func, after = $after);
        )+
    };
}

#[macro_export]
macro_rules! static_commands_with_default {
    ($macro_to_call:ident! ( $($name:ident, $doc:literal,)* )) => {
        $macro_to_call! {
        vim_visual_lines, "visual lines (vim)",
        vim_normal_mode, "visual lines (vim)",
        vim_exit_select_mode, "visual lines (vim)",
        vim_move_next_word_start, "damn",
        vim_move_next_long_word_start, "damn",
        vim_extend_next_word_start, "vim",
        vim_extend_next_long_word_start, "vim",
        vim_extend_visual_line_up, "vim",
        vim_extend_visual_line_down, "vim",
            $($name, $doc,)*
        }
    };
}

pub use vim_commands::*;

mod vim_commands {
    use vim_patch::exit_select_mode;

    use super::*;

    pub fn vim_visual_lines(cx: &mut Context) {
        select_mode(cx);
        VIM_STATE.visual_line();
        extend_to_line_bounds(cx);
    }

    wrap_many_with_hooks!(
        [
            (vim_move_next_word_start, move_next_word_start),
            (vim_move_next_long_word_start, move_next_long_word_start),
        ],
        after = move_char_right
    );

    wrap_many_with_hooks!(
        [
            (vim_extend_next_word_start, extend_next_word_start),
            (vim_extend_next_long_word_start, extend_next_long_word_start),
        ],
        after = extend_char_right
    );

    pub fn vim_extend_visual_line_down(cx: &mut Context) {
        if VIM_STATE.is_visual_line() {
            extend_line_down(cx);
        } else {
            extend_visual_line_down(cx);
        }
    }

    pub fn vim_extend_visual_line_up(cx: &mut Context) {
        if VIM_STATE.is_visual_line() {
            extend_line_up(cx);
        } else {
            extend_visual_line_up(cx);
        }
    }

    pub fn vim_normal_mode(cx: &mut Context) {
        normal_mode(cx);
        VIM_STATE.exit_visual_line();
    }

    pub fn vim_exit_select_mode(cx: &mut Context) {
        exit_select_mode(cx);
        VIM_STATE.exit_visual_line();
    }
}

#[derive(Copy, Clone, Debug, PartialEq, Eq)]
pub enum EvilOperator {
    Yank,
    Delete,
    Change,
}

pub struct EvilOps;

impl EvilOps {
    fn strip_trailing_line_break(text: &Rope, range: (usize, usize)) -> (usize, usize) {
        let start = range.0.min(range.1);
        let mut end = range.0.max(range.1);
        let inversed = range.0 > range.1;

        // The end points to the next char, not to the last char which would be selected
        if end.saturating_sub(start) >= 1 && text.char(end - 1) == '\n' {
            end -= 1;

            // The line might end with CR & LF; in that case, strip CR as well
            if end.saturating_sub(start) >= 1 && text.char(end - 1) == '\r' {
                end -= 1;
            }
        }

        return if !inversed {
            (start, end)
        } else {
            (end, start)
        };
    }

    fn get_full_line_based_selection(
        cx: &mut Context,
        count: usize,
        include_final_line_break: bool,
    ) -> Selection {
        let lines_to_select = count;
        let (view, doc) = current!(cx.editor);

        let text = doc.text();
        let extend = Extend::Below;

        log::trace!("Calculating full line-based selection (lines to select: {}, extend below: {}, include final line break: {})", lines_to_select, match extend {
            Extend::Above => false,
            Extend::Below => true,
        }, include_final_line_break);

        // Process a number of lines: first create a temporary selection of the text to be processed
        return doc.selection(view.id).clone().transform(|range| {
            let (start_line, end_line) = range.line_range(text.slice(..));

            let start: usize = text.line_to_char(start_line);
            let end: usize = text.line_to_char((end_line + lines_to_select).min(text.len_lines()));

            // Extend to previous/next line if current line is selected
            let (mut anchor, mut head) = if range.from() == start && range.to() == end {
                match extend {
                    Extend::Above => (end, text.line_to_char(start_line.saturating_sub(1))),
                    Extend::Below => (
                        start,
                        text.line_to_char((end_line + lines_to_select).min(text.len_lines())),
                    ),
                }
            } else {
                (start, end)
            };

            // Strip the final line break if requested
            if !include_final_line_break {
                (anchor, head) = Self::strip_trailing_line_break(text, (anchor, head));
            }

            Range::new(anchor, head)
        });
    }

    fn strip_last_grapheme_within_line(text: RopeSlice, range: Range) -> Range {
        let line = text.char_to_line(range.anchor);
        let line_start = text.line_to_char(line);
        let line_end = line_end_char_index(&text, line).max(line_start);

        let new_head = if line_end > range.head {
            prev_grapheme_boundary(text, range.head).max(line_start)
        } else {
            line_end
        };

        Range::new(range.anchor, new_head)
    }

    fn yank_selection(
        cx: &mut Context,
        selection: &Selection,
        register: Option<char>,
        _set_status_message: bool,
    ) {
        // Copy/paste of `yank` and `yank_impl` from commands.rs.
        let editor = &mut cx.editor;
        let register = register.unwrap_or(editor.config().default_yank_register);

        let (_view, doc) = current!(editor);
        let text = doc.text().slice(..);

        let values: Vec<String> = selection.fragments(text).map(Cow::into_owned).collect();
        let _selections = values.len();

        match editor.registers.write(register, values) {
            Ok(_) => {}
            Err(err) => editor.set_error(err.to_string()),
        }
    }

    fn delete_selection_without_yank(
        cx: &mut Context,
        selection: &Selection,
        _set_status_message: bool,
    ) {
        let (view, doc) = current!(cx.editor);
        let transaction = Transaction::change_by_selection(doc.text(), selection, |range| {
            (range.from(), range.to(), None)
        });

        doc.apply(&transaction, view.id);
    }

    fn exit_selection(cx: &mut Context) {
        // Self::collapse_selections(cx, collapse_mode);
        exit_select_mode(cx);

        if cx.editor.mode == Mode::Select {
            cx.editor.mode = Mode::Normal;
        }
    }

    fn run_operator(
        cx: &mut Context,
        cmd: EvilOperator,
        register: Option<char>,
        selection_to_yank: &Selection,
        selection_to_delete: &Selection,
    ) {
        // Self::stop_pending_and_collapse_to_anchor(cx);

        Self::yank_selection(cx, selection_to_yank, register, true);

        match cmd {
            EvilOperator::Delete | EvilOperator::Change => {
                Self::delete_selection_without_yank(cx, selection_to_delete, true);
            }
            _ => return,
        }

        if cmd == EvilOperator::Change {
            insert_mode(cx);
        }
    }

    fn run_operator_for_current_selection(
        cx: &mut Context,
        cmd: EvilOperator,
        register: Option<char>,
        special_case: Option<EvilOpsCase>,
    ) {
        let (view, doc) = current!(cx.editor);
        let selection = doc.selection(view.id).clone();
        match special_case {
            Some(EvilOpsCase::NextWord) => {
                let text = doc.text().slice(..);
                let delete_selection = selection
                    .clone()
                    .transform(|range| Self::strip_last_grapheme_within_line(text, range));
                Self::run_operator(cx, cmd, register, &selection, &delete_selection);
            }
            Some(EvilOpsCase::CompleteLines) => {
                Self::run_operator_lines(cx, cmd, register, cx.count());
            }
            None => {
                Self::run_operator(cx, cmd, register, &selection, &selection);
            }
        }
    }

    fn run_operator_lines(cx: &mut Context, cmd: EvilOperator, register: Option<char>, count: usize) {
        let selection = Self::get_full_line_based_selection(cx, count, true);
        if cmd != EvilOperator::Change {
            Self::run_operator(cx, cmd, register, &selection, &selection);
        } else {
            let delete_selection = Self::get_full_line_based_selection(cx, count, false);
            Self::run_operator(cx, cmd, register, &selection, &delete_selection);
        }
    }

    // fn invalid_selection_change(cx: &mut Context, prev_sel: &Selection, new_sel: &Selection) -> bool {
    //     fn is_invalid(sel1: &Selection, sel2: &Selection) -> bool {
    //         // Note: different `selection.len()` is invalid, returns true independent of ranges
    //         sel1.len() != sel2.len()
    //             || sel1
    //                 .ranges()
    //                 .iter()
    //                 .zip(sel2.ranges())
    //                 .all(|(r1, r2)| r1.anchor == r2.anchor && r1.head == r2.head)
    //     }

    //     let (view, doc) = current!(cx.editor);
    //     let current_selection = doc.selection(view.id);
    //     is_invalid(prev_sel, new_sel)
    // }

    pub fn operator_impl(cx: &mut Context, cmd: EvilOperator, register: Option<char>) {
        // Case example: dd, yy, cc
        let count = cx.count();

        cx.on_next_key(move |cx, event| {
            cx.editor.autoinfo = None;
            if let Some(ch) = event.char() {
                match ch {
                    'i' => vim_operate_textobject(cx, textobject::TextObject::Inside, cmd),
                    'a' => vim_operate_textobject(cx, textobject::TextObject::Inside, cmd),
                    'd' => EvilOps::run_operator_lines(cx, cmd, register, count),
                    'y' => EvilOps::run_operator_lines(cx, cmd, register, count),
                    'c' => EvilOps::run_operator_lines(cx, cmd, register, count),
                    _ => (),
                    // 'W' => textobject::textobject_word(text, range, objtype, count, true),
                    // 't' => textobject_treesitter("class", range),
                    // 'f' => textobject_treesitter("function", range),
                }
            }
        })
    }
}

fn vim_operate_textobject(cx: &mut Context, objtype: textobject::TextObject, op: EvilOperator) {
    // adapted from select_textobject
    let count = cx.count();

    cx.on_next_key(move |cx, event| {
        cx.editor.autoinfo = None;
        if let Some(ch) = event.char() {
            let (view, doc) = current!(cx.editor);

            let loader = cx.editor.syn_loader.load();
            let text = doc.text().slice(..);

            let textobject_treesitter = |obj_name: &str, range: Range| -> Range {
                let Some(syntax) = doc.syntax() else {
                    return range;
                };
                textobject::textobject_treesitter(
                    text, range, objtype, obj_name, syntax, &loader, count,
                )
            };

            let textobject_change = |range: Range| -> Range {
                let diff_handle = doc.diff_handle().unwrap();
                let diff = diff_handle.load();
                let line = range.cursor_line(text);
                let hunk_idx = if let Some(hunk_idx) = diff.hunk_at(line as u32, false) {
                    hunk_idx
                } else {
                    return range;
                };
                let hunk = diff.nth_hunk(hunk_idx).after;

                let start = text.line_to_char(hunk.start as usize);
                let end = text.line_to_char(hunk.end as usize);
                Range::new(start, end).with_direction(range.direction())
            };
            let mut is_valid = true;
            let selection = doc.selection(view.id).clone().transform(|range| {
                match ch {
                    'w' => textobject::textobject_word(text, range, objtype, count, false),
                    'W' => textobject::textobject_word(text, range, objtype, count, true),
                    't' => textobject_treesitter("class", range),
                    'f' => textobject_treesitter("function", range),
                    'a' => textobject_treesitter("parameter", range),
                    'c' => textobject_treesitter("comment", range),
                    'T' => textobject_treesitter("test", range),
                    'e' => textobject_treesitter("entry", range),
                    'p' => textobject::textobject_paragraph(text, range, objtype, count),
                    'm' => textobject::textobject_pair_surround_closest(
                        doc.syntax(),
                        text,
                        range,
                        objtype,
                        count,
                    ),
                    'g' => textobject_change(range),
                    // TODO: cancel new ranges if inconsistent surround matches across lines
                    ch if !ch.is_ascii_alphanumeric() => textobject::textobject_pair_surround(
                        doc.syntax(),
                        text,
                        range,
                        objtype,
                        ch,
                        count,
                    ),
                    _ => {
                        is_valid = false;
                        range
                    }
                }
            });
            if is_valid {
                EvilOps::run_operator(cx, op, cx.register, &selection, &selection);
            }
        }
    });

    let title = match objtype {
        textobject::TextObject::Inside => "Match inside",
        textobject::TextObject::Around => "Match around",
        _ => return,
    };
    let help_text = [
        ("w", "Word"),
        ("W", "WORD"),
        ("p", "Paragraph"),
        ("t", "Type definition (tree-sitter)"),
        ("f", "Function (tree-sitter)"),
        ("a", "Argument/parameter (tree-sitter)"),
        ("c", "Comment (tree-sitter)"),
        ("T", "Test (tree-sitter)"),
        ("e", "Data structure entry (tree-sitter)"),
        ("m", "Closest surrounding pair (tree-sitter)"),
        ("g", "Change"),
        (" ", "... or any character acting as a pair"),
    ];

    cx.editor.autoinfo = Some(Info::new(title, &help_text));
}
