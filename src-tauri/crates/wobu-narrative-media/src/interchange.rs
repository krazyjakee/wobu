//! Deterministic UTF-8 JSON or RFC4180 CSV, one row per locale/string/plural form.
use crate::{Result, Row, invalid};
const HEADERS: [&str; 18] = [
    "version",
    "id",
    "locale",
    "form",
    "speaker",
    "text",
    "source_revision",
    "source_guard",
    "translation_guard",
    "pronunciation",
    "delivery",
    "parameters",
    "audio_path",
    "timing_path",
    "audio_hash",
    "timing_hash",
    "media_guard",
    "frozen_source",
];
pub fn encode(rows: &[Row], csv: bool) -> Result<String> {
    let mut rows = rows.to_vec();
    rows.sort_by_key(|row| row.key.token());
    if !csv {
        return serde_json::to_string_pretty(&rows).map_err(|e| invalid(e.to_string()));
    }
    let mut writer =
        csv::WriterBuilder::new().terminator(csv::Terminator::CRLF).from_writer(Vec::new());
    writer.write_record(HEADERS).map_err(|e| invalid(e.to_string()))?;
    for row in rows {
        let mut cells = frozen_cells(&row).to_vec();
        cells.extend([
            serde_json::to_string(&row.parameters).map_err(|e| invalid(e.to_string()))?,
            row.audio_path.clone(),
            row.timing_path.clone().unwrap_or_default(),
            row.audio_hash.clone().unwrap_or_default(),
            row.timing_hash.clone().unwrap_or_default(),
            row.media_guard.clone().unwrap_or_default(),
            serde_json::to_string(&row).map_err(|e| invalid(e.to_string()))?,
        ]);
        writer.write_record(cells).map_err(|e| invalid(e.to_string()))?;
    }
    String::from_utf8(writer.into_inner().map_err(|e| invalid(e.to_string()))?)
        .map_err(|e| invalid(e.to_string()))
}
pub fn decode(input: &str, csv: bool) -> Result<Vec<Row>> {
    if input.len() > 16 * 1024 * 1024 {
        return Err(invalid("Recording manifest exceeds 16 MiB."));
    }
    let input = input.strip_prefix('\u{feff}').unwrap_or(input);
    let rows: Vec<Row> = if csv {
        let mut reader = csv::ReaderBuilder::new().from_reader(input.as_bytes());
        if reader.headers().map_err(|e| invalid(e.to_string()))?.iter().ne(HEADERS) {
            return Err(invalid("Preserve exported CSV columns in order."));
        }
        let mut out = Vec::new();
        for record in reader.records() {
            let record = record.map_err(|e| invalid(e.to_string()))?;
            let get = |i| record.get(i).ok_or_else(|| invalid("Missing CSV cell."));
            let mut row: Row =
                serde_json::from_str(get(17)?).map_err(|e| invalid(e.to_string()))?;
            let frozen = frozen_cells(&row);
            for (i, value) in frozen.iter().enumerate() {
                if get(i)? != value {
                    return Err(invalid(
                        "Frozen script columns changed; export again after editing saved recording notes or wording.",
                    ));
                }
            }
            row.parameters = serde_json::from_str(get(11)?).map_err(|e| invalid(e.to_string()))?;
            row.audio_path = get(12)?.into();
            row.timing_path = (!get(13)?.is_empty()).then(|| get(13).unwrap().to_string());
            row.audio_hash = (!get(14)?.is_empty()).then(|| get(14).unwrap().to_string());
            row.timing_hash = (!get(15)?.is_empty()).then(|| get(15).unwrap().to_string());
            if get(16)? != row.media_guard.clone().unwrap_or_default() {
                return Err(invalid("Media guard was edited."));
            }
            out.push(row);
        }
        out
    } else {
        serde_json::from_str(input).map_err(|e| invalid(e.to_string()))?
    };
    if rows.len() > 100_000 {
        return Err(invalid("Recording manifest exceeds 100,000 rows."));
    }
    Ok(rows)
}

fn frozen_cells(row: &Row) -> [String; 11] {
    [
        row.version.to_string(),
        row.key.id.clone(),
        row.key.locale.to_string(),
        row.key.form_name(),
        row.source.speaker.clone(),
        row.text.clone(),
        row.source.revision.clone(),
        row.source.guard.clone(),
        row.translation_guard.clone().unwrap_or_default(),
        row.notes.pronunciation.clone(),
        row.notes.delivery.clone(),
    ]
}
