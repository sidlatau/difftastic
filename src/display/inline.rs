//! Inline, or "unified" diff display.

use line_numbers::LineNumber;

use crate::constants::Side;
use crate::display::context::all_matched_lines_filled;
use crate::display::diff_line_metadata::{row_is_novel, DiffLineMetadata};
use crate::display::hunks::{matched_lines_indexes_for_hunk, Hunk};
use crate::display::side_by_side::{highlight_as_novel, hunk_first_new_line, lines_with_novel};
use crate::display::style::{self, apply_colors, apply_line_number_color};
use crate::lines::{format_line_num, format_line_num_padded, split_on_newlines, MaxLine};
use crate::options::DisplayOptions;
use crate::parse::syntax::MatchedPos;
use crate::summary::FileFormat;

pub(crate) fn print(
    lhs_src: &str,
    rhs_src: &str,
    display_options: &DisplayOptions,
    lhs_mps: &[MatchedPos],
    rhs_mps: &[MatchedPos],
    hunks: &[Hunk],
    display_path: &str,
    extra_info: &Option<String>,
    file_format: &FileFormat,
) {
    let (lhs_colored_lines, rhs_colored_lines) = if display_options.use_color {
        (
            apply_colors(
                lhs_src,
                Side::Left,
                display_options.syntax_highlight,
                file_format,
                display_options.background_color,
                lhs_mps,
            ),
            apply_colors(
                rhs_src,
                Side::Right,
                display_options.syntax_highlight,
                file_format,
                display_options.background_color,
                rhs_mps,
            ),
        )
    } else {
        (
            split_on_newlines(lhs_src)
                .map(|s| format!("{}\n", s))
                .collect(),
            split_on_newlines(rhs_src)
                .map(|s| format!("{}\n", s))
                .collect(),
        )
    };

    let lhs_colored_lines: Vec<_> = lhs_colored_lines
        .into_iter()
        .map(|line| style::replace_tabs(&line, display_options.tab_width))
        .collect();
    let rhs_colored_lines: Vec<_> = rhs_colored_lines
        .into_iter()
        .map(|line| style::replace_tabs(&line, display_options.tab_width))
        .collect();

    let metadata = DiffLineMetadata::from_env(display_path);

    let mut lhs_lines: Vec<&str> = split_on_newlines(lhs_src).collect();
    let mut rhs_lines: Vec<&str> = split_on_newlines(rhs_src).collect();
    if lhs_lines.last() == Some(&"") && lhs_lines.len() > 1 {
        lhs_lines.pop();
    }
    if rhs_lines.last() == Some(&"") && rhs_lines.len() > 1 {
        rhs_lines.pop();
    }

    let (lhs_lines_with_novel, rhs_lines_with_novel) = lines_with_novel(lhs_mps, rhs_mps);
    let matched_lines = all_matched_lines_filled(lhs_mps, rhs_mps, &lhs_lines, &rhs_lines);
    let mut matched_lines_to_print = &matched_lines[..];

    let lhs_line_nums_width = format_line_num(lhs_src.max_line()).len();
    let rhs_line_nums_width = format_line_num(rhs_src.max_line()).len();

    // Every row has both line-number columns, blank where that side has no
    // line, so the content starts in the same column on every row.
    let gutter = |lhs: Option<(LineNumber, bool)>, rhs: Option<(LineNumber, bool)>| {
        let lhs = match lhs {
            Some((line_num, is_novel)) => apply_line_number_color(
                &format_line_num_padded(line_num, lhs_line_nums_width),
                is_novel,
                Side::Left,
                display_options,
            ),
            None => " ".repeat(lhs_line_nums_width),
        };
        let rhs = match rhs {
            Some((line_num, is_novel)) => apply_line_number_color(
                &format_line_num_padded(line_num, rhs_line_nums_width),
                is_novel,
                Side::Right,
                display_options,
            ),
            None => " ".repeat(rhs_line_nums_width),
        };
        format!("{}{}", lhs, rhs)
    };

    let mut prev_rhs: Option<LineNumber> = None;

    for (i, hunk) in hunks.iter().enumerate() {
        let (start_i, end_i) = matched_lines_indexes_for_hunk(
            matched_lines_to_print,
            hunk,
            display_options.num_context_lines as usize,
        );
        let aligned_lines = &matched_lines_to_print[start_i..end_i];
        matched_lines_to_print = &matched_lines_to_print[start_i..];

        let banner = style::header(
            display_path,
            extra_info.as_ref(),
            i + 1,
            hunks.len(),
            file_format,
            display_options,
        );
        match &metadata {
            // The banner announces both the file and this hunk: every banner
            // carries the hunk's `h` (with the hunk's first new-file line), and
            // the first hunk's banner additionally carries the file's `f`.
            Some(metadata) => {
                let new_line = if rhs_src.is_empty() {
                    0
                } else {
                    hunk_first_new_line(aligned_lines, prev_rhs)
                };
                println!("{}", metadata.header_banner(i == 0, new_line, &banner));
            }
            None => println!("{}", banner),
        }

        // Rows come aligned as in side-by-side display. An unchanged row
        // prints once; a run of changed rows prints all its old lines, then
        // all its new lines, like a unified diff.
        let mut removed: Vec<String> = vec![];
        let mut added: Vec<String> = vec![];

        for &(lhs_line, rhs_line) in aligned_lines {
            // An empty file still has one (empty) line to align against,
            // which is not content to show.
            let lhs_line = lhs_line.filter(|_| !lhs_src.is_empty());
            let rhs_line = rhs_line.filter(|_| !rhs_src.is_empty());

            let lhs_novel =
                highlight_as_novel(lhs_line, &lhs_lines, rhs_line, &lhs_lines_with_novel);
            let rhs_novel =
                highlight_as_novel(rhs_line, &rhs_lines, lhs_line, &rhs_lines_with_novel);

            // The line type is the row's patch-space type (contents compared
            // -- see row_is_novel), not the token novelty the display colors
            // by: a line changed only by added tokens has no novel old-side
            // tokens, yet its old line is still the deletion half of the
            // modification.
            let (lhs_osc, rhs_osc) = match &metadata {
                Some(metadata) => {
                    let row_novel = row_is_novel(lhs_line, rhs_line, &lhs_lines, &rhs_lines);
                    // A deleted file has no new-file content, so its
                    // deletions sit at new-line 0, as in single-column display.
                    let lhs_osc = match lhs_line {
                        Some(lhs_line) if rhs_src.is_empty() => {
                            metadata.single_column_cell(Side::Left, lhs_line.as_usize())
                        }
                        _ => metadata.left_cell(lhs_line, rhs_line, row_novel, prev_rhs),
                    };
                    (lhs_osc, metadata.right_cell(rhs_line, row_novel))
                }
                None => (String::new(), String::new()),
            };

            match (lhs_line, rhs_line) {
                (Some(lhs_line), Some(rhs_line)) if !lhs_novel && !rhs_novel => {
                    print_change(&mut removed, &mut added);
                    // A row whose sides differ only in whitespace prints its
                    // new line but still carries its d and a records.
                    let row_oscs = if lhs_osc == rhs_osc {
                        lhs_osc
                    } else {
                        format!("{}{}", lhs_osc, rhs_osc)
                    };
                    print!(
                        "{}{}{}",
                        row_oscs,
                        gutter(Some((lhs_line, false)), Some((rhs_line, false))),
                        rhs_colored_lines[rhs_line.as_usize()]
                    );
                }
                _ => {
                    if let Some(lhs_line) = lhs_line {
                        removed.push(format!(
                            "{}{}{}",
                            lhs_osc,
                            gutter(Some((lhs_line, lhs_novel)), None),
                            lhs_colored_lines[lhs_line.as_usize()]
                        ));
                    }
                    if let Some(rhs_line) = rhs_line {
                        added.push(format!(
                            "{}{}{}",
                            rhs_osc,
                            gutter(None, Some((rhs_line, rhs_novel))),
                            rhs_colored_lines[rhs_line.as_usize()]
                        ));
                    }
                }
            }

            if rhs_line.is_some() {
                prev_rhs = rhs_line;
            }
        }
        print_change(&mut removed, &mut added);
        println!();
    }
}

fn print_change(removed: &mut Vec<String>, added: &mut Vec<String>) {
    for line in removed.drain(..).chain(added.drain(..)) {
        print!("{}", line);
    }
}
