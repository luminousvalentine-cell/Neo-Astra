use crate::ReportedDiagnostic;

pub fn snippet(text: &str, start: usize, end: usize, line: usize) -> Option<String> {
    if start > text.len() {
        return None;
    }
    let end = end.min(text.len()).max(start);
    let line_start = text[..start].rfind('\n').map_or(0, |index| index + 1);
    let line_end = text[start..]
        .find('\n')
        .map_or(text.len(), |index| start + index);
    let source_line = text[line_start..line_end].trim_end_matches('\r');
    let column = text[line_start..start].chars().count();
    let carets = text[start..end.min(line_end)].chars().count().max(1);
    let gutter = " ".repeat(line.to_string().len());
    Some(format!(
        "{gutter} |\n{line:>width$} | {source_line}\n{gutter} | {padding}{carets}",
        gutter = gutter,
        line = line,
        width = gutter.len(),
        source_line = source_line,
        padding = " ".repeat(column),
        carets = "^".repeat(carets),
    ))
}

pub fn render(diagnostic: &ReportedDiagnostic) -> String {
    let gutter = " ".repeat(diagnostic.line.map_or(1, |line| line.to_string().len()));
    let mut out = format!(
        "{}[{}]: {}",
        diagnostic.severity, diagnostic.code, diagnostic.message
    );
    if let Some(path) = diagnostic.path.as_deref() {
        out.push('\n');
        out.push_str(&gutter);
        out.push_str("--> ");
        out.push_str(path);
        if let Some(line) = diagnostic.line {
            out.push_str(&format!(":{}:{}", line, diagnostic.column.unwrap_or(1)));
        }
    }
    if let Some(snippet) = diagnostic.snippet.as_deref() {
        out.push('\n');
        out.push_str(snippet);
    }
    if let Some(hint) = diagnostic.hint.as_deref() {
        if diagnostic.snippet.is_some() {
            out.push('\n');
            out.push_str(&gutter);
            out.push_str(" |");
        }
        out.push('\n');
        out.push_str(&gutter);
        out.push_str(" = help: ");
        out.push_str(hint);
    }
    for related in &diagnostic.related {
        out.push_str("\nnote: ");
        out.push_str(&related.message);
        out.push('\n');
        out.push_str(&gutter);
        out.push_str("--> ");
        out.push_str(&related.file);
        if let Some(line) = related.line {
            out.push_str(&format!(":{}:{}", line, related.column.unwrap_or(1)));
        }
    }
    out
}

pub fn summary(diagnostics: &[ReportedDiagnostic]) -> Option<String> {
    let errors = diagnostics
        .iter()
        .filter(|diagnostic| diagnostic.severity == "error")
        .count();
    match errors {
        0 => None,
        1 => Some("error: aborting due to previous error".to_owned()),
        count => Some(format!("error: aborting due to {count} previous errors")),
    }
}
