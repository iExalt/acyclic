//! Output for people, from the same data the JSON envelope carries: tables for lists, a block of fields for each
//! record, and one line for a write. Fitted to the terminal's width when printing to one, and without borders or
//! wrapping when not, so it stays easy to pipe.

use std::fmt::Write as _;

use comfy_table::{ContentArrangement, LineStyle, Table, TableStyle, presets};
use serde_json::Value as Json;

use crate::args::{Command, LinkCommand, ThreadCommand};

/// A terminal's table: a rule under the header and nothing else, so a wrapped cell still reads as one row.
const RULED: TableStyle =
    TableStyle::new().header_separator(LineStyle::none().fill('─').junction('─'));

/// The entities a write names, in the order a summary looks for them.
const ENTITIES: [&str; 4] = ["item", "thread", "repository", "proposal"];

/// A command's data for people. `width` is the terminal's, or `None` when not printing to one.
pub(crate) fn render(command: &Command, data: &Json, width: Option<u16>) -> String {
    let columns = |keys: &[&str]| keys.iter().map(|key| (*key).to_owned()).collect::<Vec<_>>();
    let text = match command {
        Command::List => table(
            rows(&data["items"]),
            &columns(&["id", "title", "state", "thread", "readiness"]),
            width,
        ),
        Command::Ready { .. } => ready(data, width),
        Command::Thread(ThreadCommand::List) => threads(data, width),
        Command::Import { .. } => import(data),
        Command::Link(LinkCommand::Add { .. } | LinkCommand::Remove { .. }) => {
            let changed = if data["changed"] == true { "changed" } else { "no change to" };
            let [from, kind, to] = ["from", "kind", "to"].map(|key| compact(&data[key]));
            format!("{changed} link {from} {kind} {to}")
        },
        _ => match data {
            Json::Array(records) => {
                records.iter().map(|record| block(record, width)).collect::<Vec<_>>().join("\n")
            },
            Json::Object(fields) if fields.contains_key("changed") => write(data),
            _ => block(data, width),
        },
    };
    if text.ends_with('\n') { text } else { format!("{text}\n") }
}

/// An error's envelope as one line for standard error, with its details when it has any, since they often name what
/// to act on.
pub(crate) fn error(envelope: &Json) -> String {
    let error = &envelope["error"];
    let message = error["message"].as_str().unwrap_or_default();
    let code = error["code"].as_str().unwrap_or_default();
    let details = &error["details"];
    if empty(details) {
        format!("error: {message} [{code}]\n")
    } else {
        format!("error: {message} [{code}] ({})\n", compact(details))
    }
}

/// An import as one line: what it wrote, or would write on a dry run, by kind, and what waits as a proposal.
fn import(data: &Json) -> String {
    let counts = &data["counts"];
    let kinds = [
        ("items", "item"),
        ("threads", "thread"),
        ("links", "link"),
        ("repositories", "repository"),
    ];
    let written: Vec<String> = kinds
        .iter()
        .filter_map(|(key, one)| {
            let n = counts[*key].as_u64().filter(|n| *n > 0)?;
            Some(if n == 1 { format!("1 {one}") } else { format!("{n} {key}") })
        })
        .collect();
    let mut line = match (written.is_empty(), data["dry_run"] == true) {
        (true, _) => "no change".to_owned(),
        (false, dry) => format!(
            "{}: {} ({} fields)",
            if dry { "would import" } else { "imported" },
            written.join(", "),
            compact(&counts["fields"])
        ),
    };
    for key in ["proposals", "placed"] {
        if !empty(&data[key]) {
            write!(line, " · {key} {}", compact(&data[key])).expect("a String takes writes");
        }
    }
    line
}

/// The objects in a list.
fn rows(list: &Json) -> Vec<&Json> {
    list.as_array().map(|rows| rows.iter().collect()).unwrap_or_default()
}

/// Rows as a table under the given columns, or `none` for no rows.
fn table(rows: Vec<&Json>, columns: &[String], width: Option<u16>) -> String {
    if rows.is_empty() {
        return "none\n".to_owned();
    }
    let mut table = Table::new();
    match width {
        Some(width) => {
            table
                .load_style(RULED)
                .set_content_arrangement(ContentArrangement::Dynamic)
                .set_width(width);
        },
        None => {
            table
                .load_style(presets::NOTHING)
                .set_content_arrangement(ContentArrangement::Disabled);
        },
    }
    table.set_header(columns.iter().map(|column| column.to_uppercase()));
    for row in rows {
        table.add_row(columns.iter().map(|column| compact(&row[column])));
    }
    let mut text = table.to_string();
    if width.is_none() {
        // Without borders, the padding at each line's end is noise in a pipe.
        text = text
            .lines()
            .map(|line| line.strip_prefix(' ').unwrap_or(line).trim_end())
            .collect::<Vec<_>>()
            .join("\n");
    }
    text.push('\n');
    text
}

/// `ready`: one row per ready item, under its thread, the thread named on its first row.
fn ready(data: &Json, width: Option<u16>) -> String {
    let mut lines = Vec::new();
    for thread in rows(&data["ready"]) {
        let mut first = true;
        for item in rows(&thread["items"]) {
            let named = if std::mem::take(&mut first) {
                format!("{} {}", compact(&thread["id"]), compact(&thread["title"]))
            } else {
                String::new()
            };
            lines.push(serde_json::json!({
                "thread": named, "item": item["id"], "title": item["title"], "approved": item["approved"],
            }));
        }
    }
    let columns: Vec<String> =
        ["thread", "item", "title", "approved"].iter().map(|key| (*key).to_owned()).collect();
    table(lines.iter().collect(), &columns, width)
}

/// `thread list`: each thread's summary, with its counts as columns.
fn threads(data: &Json, width: Option<u16>) -> String {
    let lines: Vec<Json> = rows(&data["threads"])
        .into_iter()
        .map(|thread| {
            serde_json::json!({
                "id": thread["id"], "title": thread["title"], "approved": thread["approved"],
                "members": thread["counts"]["members"], "done": thread["counts"]["done"], "heads": thread["heads"],
            })
        })
        .collect();
    let columns: Vec<String> = ["id", "title", "approved", "members", "done", "heads"]
        .iter()
        .map(|key| (*key).to_owned())
        .collect();
    table(lines.iter().collect(), &columns, width)
}

/// A write as one line: what changed, and anything else it says that isn't empty.
fn write(data: &Json) -> String {
    let fields = data.as_object().expect("a write's data is an object");
    let entity = ENTITIES.iter().find(|entity| fields.get(**entity).is_some_and(Json::is_string));
    let changed = if data["changed"] == true { "changed" } else { "no change to" };
    let mut parts = vec![match entity {
        Some(entity) => format!("{changed} {entity} {}", compact(&data[*entity])),
        None => changed.to_owned(),
    }];
    for (key, value) in fields {
        if key == "changed" || entity.is_some_and(|entity| *entity == key.as_str()) || empty(value)
        {
            continue;
        }
        parts.push(format!("{key} {}", compact(value)));
    }
    parts.join(" · ")
}

/// A record as one field per line, its names aligned, leaving out what is empty. A list of records inside it is a
/// table under its name.
fn block(record: &Json, width: Option<u16>) -> String {
    let Some(fields) = record.as_object() else {
        return compact(record);
    };
    // A document that is only a list, as `needs` and `proposal list` are, is that list's table.
    if let [(_, list)] = fields.iter().collect::<Vec<_>>()[..]
        && (list.is_array())
    {
        return list_table(rows(list), width);
    }
    let shown: Vec<(&String, &Json)> = leading(fields.keys())
        .into_iter()
        .map(|key| (key, &fields[key]))
        .filter(|(_, value)| !empty(value))
        .collect();
    let (lists, scalars): (Vec<_>, Vec<_>) =
        shown.into_iter().partition(|(_, value)| records(value));
    let name_width = scalars.iter().map(|(key, _)| key.len()).max().unwrap_or(0);
    let mut text = String::new();
    for (key, value) in scalars {
        // A value of several lines, such as notes, keeps its later lines under its first.
        let value =
            compact(value).replace('\n', &format!("\n{:indent$}", "", indent = name_width + 2));
        writeln!(text, "{key:name_width$}  {value}").expect("a String takes writes");
    }
    for (key, value) in lists {
        write!(text, "\n{key}:\n{}", list_table(rows(value), width))
            .expect("a String takes writes");
    }
    text
}

/// Records of any shape as a table, with a column for every field any of them has.
fn list_table(rows: Vec<&Json>, width: Option<u16>) -> String {
    let mut keys: Vec<&String> = Vec::new();
    for row in &rows {
        for column in row.as_object().into_iter().flat_map(serde_json::Map::keys) {
            if !keys.contains(&column) {
                keys.push(column);
            }
        }
    }
    let columns: Vec<String> = leading(keys).into_iter().cloned().collect();
    table(rows, &columns, width)
}

/// Field names with `kind`, `id` and `title` first, as a record is named, then the rest in order.
fn leading<'a>(keys: impl IntoIterator<Item = &'a String>) -> Vec<&'a String> {
    let mut keys: Vec<&String> = keys.into_iter().collect();
    keys.sort_by_key(|key| match key.as_str() {
        "kind" => 0,
        "id" => 1,
        "title" => 2,
        _ => 3,
    });
    keys
}

/// Whether a value is a non-empty list of records.
fn records(value: &Json) -> bool {
    value.as_array().is_some_and(|values| !values.is_empty() && values.iter().all(Json::is_object))
}

/// Whether a value says nothing: null, or an empty list, record or text.
fn empty(value: &Json) -> bool {
    match value {
        Json::Null => true,
        Json::String(text) => text.is_empty(),
        Json::Array(values) => values.is_empty(),
        Json::Object(fields) => fields.values().all(empty),
        _ => false,
    }
}

/// A value on one line: text as it is, a list joined with commas, an embedded record by its `id`, and another
/// record's fields that say something as `name value`, its `status` first and alone, as a readiness reads.
fn compact(value: &Json) -> String {
    match value {
        Json::Null => String::new(),
        Json::Bool(yes) => (if *yes { "yes" } else { "no" }).to_owned(),
        Json::String(text) => text.clone(),
        Json::Number(number) => number.to_string(),
        // An embedded record reads as its id, with its one other field when it has only one, as a proposal's does.
        Json::Object(fields) if fields.get("id").is_some_and(Json::is_string) => {
            let id = compact(&fields["id"]);
            match (fields.len(), fields.iter().find(|(key, _)| *key != "id")) {
                (2, Some((_, other))) if !other.is_object() => format!("{id} ({})", compact(other)),
                _ => id,
            }
        },
        Json::Array(values) => {
            let separator =
                if values.iter().any(|value| value.is_object() && value.get("id").is_none()) {
                    "; "
                } else {
                    ", "
                };
            values.iter().map(compact).collect::<Vec<_>>().join(separator)
        },
        Json::Object(fields) => {
            let rest = fields
                .iter()
                .filter(|(key, value)| *key != "status" && !empty(value))
                .map(|(key, value)| format!("{key} {}", compact(value)))
                .collect::<Vec<_>>()
                .join(", ");
            match (fields.get("status"), rest.is_empty()) {
                (Some(status), true) => compact(status),
                (Some(status), false) => format!("{} ({rest})", compact(status)),
                (None, _) => rest,
            }
        },
    }
}

#[cfg(test)]
mod tests;
