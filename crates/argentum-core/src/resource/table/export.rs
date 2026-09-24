//! [`Table`] CSV export: streamed header/row fragments plus RFC4180 escaping.

use super::Table;

impl<M> Table<M> {
    /// CSV header line for this table (labels, RFC4180 escaped, trailing
    /// newline included) — the first fragment of a streamed export.
    pub fn csv_header(&self) -> String
    where
        M: toasty::schema::Model,
    {
        let mut out = String::new();
        let headers: Vec<String> = self.columns.iter().map(|c| escape_csv(c.label())).collect();
        out.push_str(&headers.join(","));
        out.push('\n');
        out
    }

    /// CSV line for one row (cells, RFC4180 escaped, trailing newline
    /// included) — one fragment of a streamed export. Formula cells are
    /// defused per OWASP (a leading `'` is prepended when the first
    /// non-whitespace/control character is `=`, `+`, `-`, `@`, `|` or `%`,
    /// including CR/LF- or tab-led variants, GH #145) so a stored value like
    /// `=1+1` opens as text, not a live spreadsheet formula.
    pub fn csv_row(&self, row: &M) -> String
    where
        M: toasty::schema::Model,
    {
        let mut out = String::new();
        let cells: Vec<String> = self
            .columns
            .iter()
            .map(|c| escape_csv(&c.render_cell(row)))
            .collect();
        out.push_str(&cells.join(","));
        out.push('\n');
        out
    }
}

fn defuse_formula(s: &str) -> String {
    // Spreadsheets run formulas led by CR/LF/tab too (OWASP CSV
    // injection): the dangerous payload can start mid-cell after
    // leading whitespace, controls, or zero-width format characters
    // (BOM/ZWSP are neither whitespace nor control), so the
    // first-char test skips them. The `'` lands on the original
    // cell, before the payload.
    let trimmed = s.trim_start_matches(|c: char| {
        c.is_whitespace() || c.is_control() || matches!(c, '\u{FEFF}' | '\u{200B}')
    });
    if let Some(first) = trimmed.chars().next()
        && matches!(first, '=' | '+' | '-' | '@' | '|' | '%')
    {
        return format!("'{s}");
    }
    s.to_string()
}

fn escape_csv(s: &str) -> String {
    let s = defuse_formula(s);
    if s.contains(',') || s.contains('"') || s.contains('\n') || s.contains('\r') {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s
    }
}

#[cfg(test)]
mod tests {
    use topcoat::context::CxTestBuilder;

    use super::*;
    use crate::resource::TextColumn;

    #[derive(Debug, Clone, toasty::Model)]
    struct User {
        #[key]
        #[auto]
        id: uuid::Uuid,
        name: String,
    }

    #[test]
    fn csv_row_defuses_formula_cells_per_owasp() {
        let cx = CxTestBuilder::new().build();
        let csv_table = Table::<User>::r#for(&cx)
            .columns(TextColumn::r#for(User::fields().name(), |u| u.name.clone()));
        for payload in ["=1+1", "+1+1", "-1+1", "@SUM(1+1)", "|id", "%x", "  =cmd"] {
            let user = User {
                id: uuid::Uuid::nil(),
                name: payload.to_string(),
            };
            let body = csv_table.csv_row(&user);
            assert!(
                body.starts_with('\''),
                "formula payload {payload:?} must be defused with leading `'`, got {body:?}"
            );
        }
        // Plain values stay untouched; RFC4180 quoting still applies.
        let user = User {
            id: uuid::Uuid::nil(),
            name: "Ada, \"the\" first".to_string(),
        };
        let csv = csv_table.csv_row(&user);
        assert!(
            csv.contains("\"Ada, \"\"the\"\" first\""),
            "quoting broke: {csv:?}"
        );
    }

    #[test]
    fn csv_row_defuses_cr_lf_led_formula_cells() {
        // GH #145: spreadsheets treat CR/LF- and tab-led payloads as formulas
        // even when the dangerous character does not start the raw cell, so
        // the defuse test skips leading whitespace/controls. CR/LF-led cells
        // are RFC4180-quoted (they carry a newline); a tab-led cell has no
        // quote/comma/newline and stays bare — either way the `'` leads the
        // defused content.
        let cx = CxTestBuilder::new().build();
        let csv_table = Table::<User>::r#for(&cx)
            .columns(TextColumn::r#for(User::fields().name(), |u| u.name.clone()));
        for (payload, defused) in [
            ("\r=1+1", "'\r=1+1"),
            ("\n@cmd", "'\n@cmd"),
            ("\t+1+1", "'\t+1+1"),
            (" \r=HYPERLINK(1,2)", "' \r=HYPERLINK(1,2)"),
            // BOM/ZWSP are neither whitespace nor control: cover the
            // format-character gap explicitly.
            ("\u{FEFF}=1+1", "'\u{FEFF}=1+1"),
            ("\u{200B}@cmd", "'\u{200B}@cmd"),
        ] {
            let user = User {
                id: uuid::Uuid::nil(),
                name: payload.to_string(),
            };
            let csv = csv_table.csv_row(&user);
            assert!(
                csv.contains(defused),
                "CR/LF-led formula payload {payload:?} must be defused to {defused:?}, got {csv:?}"
            );
        }
    }
}
