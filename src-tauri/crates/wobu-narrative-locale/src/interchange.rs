//! UTF-8 JSON and RFC4180 CSV. One row per stable ID; plural forms are a JSON cell.
use crate::{Error, Result, Row};
const HEADERS: [&str; 14] = [
    "version",
    "locale",
    "id",
    "slot",
    "container",
    "speaker",
    "source_text",
    "source_revision",
    "source_guard",
    "context",
    "delivery_notes",
    "placeholders",
    "translation_guard",
    "forms",
];
pub fn encode(rows: &[Row], csv: bool) -> Result<String> {
    let mut rows = rows.to_vec();
    rows.sort_by(|a, b| (&a.locale, &a.source.id).cmp(&(&b.locale, &b.source.id)));
    if !csv {
        return serde_json::to_string_pretty(&rows).map_err(|e| Error::Malformed(e.to_string()));
    }
    let mut writer =
        csv::WriterBuilder::new().terminator(csv::Terminator::CRLF).from_writer(Vec::new());
    writer.write_record(HEADERS).map_err(err)?;
    for row in rows {
        writer
            .write_record([
                row.version.to_string(),
                row.locale.to_string(),
                row.source.id,
                row.source.slot,
                row.source.container,
                row.source.speaker,
                row.source.text,
                row.source.revision,
                row.source.guard,
                row.source.context,
                row.source.delivery_notes,
                serde_json::to_string(&row.source.placeholders).map_err(json_err)?,
                row.translation_guard.unwrap_or_default(),
                serde_json::to_string(&row.forms).map_err(json_err)?,
            ])
            .map_err(err)?;
    }
    String::from_utf8(writer.into_inner().map_err(|e| Error::Malformed(e.to_string()))?)
        .map_err(|e| Error::Malformed(e.to_string()))
}
pub fn decode(input: &str, csv: bool) -> Result<Vec<Row>> {
    if input.len() > 16 * 1024 * 1024 {
        return Err(Error::Malformed("Import exceeds 16 MiB.".into()));
    }
    let input = input.strip_prefix('\u{feff}').unwrap_or(input);
    let rows: Vec<Row> = if csv {
        let mut reader = csv::ReaderBuilder::new().from_reader(input.as_bytes());
        if reader.headers().map_err(err)?.iter().ne(HEADERS) {
            return Err(Error::interchange(
                1,
                "Expected the exported CSV columns in their original order.",
            ));
        }
        reader
            .records()
            .enumerate()
            .map(|(index, record)| {
                let r = record.map_err(err)?;
                let field = |i| {
                    r.get(i).ok_or_else(|| Error::interchange(index + 2, "Missing CSV column."))
                };
                Ok(Row {
                    version: field(0)?
                        .parse()
                        .map_err(|_| Error::interchange(index + 2, "Invalid version."))?,
                    locale: field(1)?.parse()?,
                    source: crate::SourceLine {
                        id: field(2)?.into(),
                        slot: field(3)?.into(),
                        container: field(4)?.into(),
                        speaker: field(5)?.into(),
                        text: field(6)?.into(),
                        revision: field(7)?.into(),
                        guard: field(8)?.into(),
                        context: field(9)?.into(),
                        delivery_notes: field(10)?.into(),
                        placeholders: serde_json::from_str(field(11)?).map_err(json_err)?,
                        ready: true,
                    },
                    translation_guard: (!field(12)?.is_empty())
                        .then(|| field(12).unwrap().to_string()),
                    forms: serde_json::from_str(field(13)?).map_err(json_err)?,
                })
            })
            .collect::<Result<_>>()?
    } else {
        serde_json::from_str(input).map_err(json_err)?
    };
    if rows.len() > 100_000 {
        return Err(Error::Malformed("Import exceeds 100,000 rows.".into()));
    }
    if rows.first().is_some_and(|first| rows.iter().any(|row| row.locale != first.locale)) {
        return Err(Error::Malformed(
            "Each interchange file must contain exactly one target locale.".into(),
        ));
    }
    Ok(rows)
}
fn err(e: csv::Error) -> Error {
    Error::Malformed(e.to_string())
}
fn json_err(e: serde_json::Error) -> Error {
    Error::Malformed(e.to_string())
}
