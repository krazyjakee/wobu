//! Static, self-contained world wiki export.
//!
//! Export is a read-only projection of canonical project data. The destination
//! is deliberately outside the project and is claimed with `create_dir`, so an
//! export can never overwrite a previous site or become another watched project
//! subtree. A marker is written first and removed last; any failure leaves an
//! obviously incomplete, user-recoverable folder and deletes nothing.

use std::collections::{BTreeSet, HashMap};
use std::fs;
use std::path::{Path, PathBuf};

use serde::Serialize;
use wobu_core::kind::kind_def;
use wobu_core::{Asset, Generation, Id, Node, SectionValue};

use crate::error::{Error, Result};
use crate::paths;

const INCOMPLETE: &str = ".wobu-export-incomplete";

#[derive(Debug)]
pub struct WikiSnapshot {
    pub(crate) root: PathBuf,
    pub(crate) project_name: String,
    pub(crate) nodes: Vec<Node>,
    pub(crate) assets: Vec<Asset>,
    pub(crate) generations: Vec<Generation>,
}

impl WikiSnapshot {
    pub(crate) fn new(
        root: PathBuf,
        project_name: String,
        nodes: Vec<Node>,
        assets: Vec<Asset>,
    ) -> Self {
        Self { root, project_name, nodes, assets, generations: Vec::new() }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct WikiExport {
    pub destination: String,
    pub node_count: usize,
    pub image_count: usize,
    pub missing_images: usize,
}

#[derive(Debug, Clone, Default)]
struct ExportMedia {
    original: Option<String>,
    thumb: Option<String>,
}

/// Render one immutable snapshot into a new directory.
///
/// `destination` must not exist. The existing parent is canonicalised before
/// comparing it to the canonical project root, which closes the symlink-parent
/// escape that a lexical `starts_with` check would leave open.
pub fn export(mut snapshot: WikiSnapshot, destination: &Path) -> Result<WikiExport> {
    // Receipts are canonical files, not index rows. Read them strictly here,
    // outside the shell's project lock, and before reserving the destination:
    // an incomplete sync copy must fail closed rather than silently disappear
    // from the Concepts gallery or leave behind an empty export folder.
    snapshot.generations = crate::generations::read_all_strict(&snapshot.root)?;

    validate_destination(&snapshot, destination)?;
    fs::create_dir(destination).map_err(|error| {
        if error.kind() == std::io::ErrorKind::AlreadyExists {
            Error::AlreadyExists(destination.to_path_buf())
        } else {
            Error::io(destination, error)
        }
    })?;

    write(
        &destination.join(INCOMPLETE),
        b"This export did not finish. Nothing in this folder is canonical project data.\n",
    )?;
    fs::create_dir(destination.join("nodes"))
        .map_err(|error| Error::io(destination.join("nodes"), error))?;
    fs::create_dir_all(destination.join("media/originals"))
        .map_err(|error| Error::io(destination.join("media/originals"), error))?;
    fs::create_dir_all(destination.join("media/thumbs"))
        .map_err(|error| Error::io(destination.join("media/thumbs"), error))?;

    let (media, image_count, missing_images) = copy_media(&snapshot, destination)?;
    write(&destination.join("site.css"), SITE_CSS.as_bytes())?;

    for node in &snapshot.nodes {
        let html = node_page(&snapshot, node, &media);
        write(&destination.join("nodes").join(format!("{}.html", node.id)), html.as_bytes())?;
    }
    write(&destination.join("graph.html"), graph_page(&snapshot).as_bytes())?;

    // Written last so a folder without an index is never mistaken for a
    // complete site. The marker is the final operation after that.
    write(&destination.join("index.html"), index_page(&snapshot, &media).as_bytes())?;
    fs::remove_file(destination.join(INCOMPLETE))
        .map_err(|error| Error::io(destination.join(INCOMPLETE), error))?;

    let completed = fs::canonicalize(destination)
        .map_err(|error| Error::io(destination, error))?
        .to_string_lossy()
        .into_owned();
    Ok(WikiExport {
        destination: completed,
        node_count: snapshot.nodes.len(),
        image_count,
        missing_images,
    })
}

fn validate_destination(snapshot: &WikiSnapshot, destination: &Path) -> Result<()> {
    if destination.as_os_str().is_empty() || destination.file_name().is_none() {
        return Err(Error::InvalidExportDestination(destination.to_path_buf()));
    }
    match fs::symlink_metadata(destination) {
        Ok(_) => return Err(Error::AlreadyExists(destination.to_path_buf())),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(Error::io(destination, error)),
    }
    let parent = destination
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let parent = fs::canonicalize(parent).map_err(|error| Error::io(parent, error))?;
    let root =
        fs::canonicalize(&snapshot.root).map_err(|error| Error::io(&snapshot.root, error))?;
    if parent.starts_with(&root) {
        return Err(Error::ExportInsideProject(destination.to_path_buf()));
    }
    Ok(())
}

fn copy_media(
    snapshot: &WikiSnapshot,
    destination: &Path,
) -> Result<(HashMap<Id, ExportMedia>, usize, usize)> {
    let wanted = wanted_assets(snapshot);
    let assets: HashMap<Id, &Asset> =
        snapshot.assets.iter().map(|asset| (asset.id, asset)).collect();
    let canonical_root =
        fs::canonicalize(&snapshot.root).map_err(|error| Error::io(&snapshot.root, error))?;
    let mut out = HashMap::new();
    let mut copied = 0;
    let mut missing = 0;

    for id in wanted {
        let Some(asset) = assets.get(&id) else {
            missing += 1;
            out.insert(id, ExportMedia::default());
            continue;
        };
        let mut exported = ExportMedia::default();
        let original = paths::from_rel_string(&snapshot.root, &asset.rel_path);
        match safe_source(&canonical_root, &original)? {
            Some(source) => {
                let extension =
                    source.extension().and_then(|value| value.to_str()).unwrap_or("img");
                let relative = format!("media/originals/{id}.{extension}");
                copy(&source, &destination.join(&relative))?;
                exported.original = Some(relative);
                copied += 1;
            }
            None => missing += 1,
        }

        if let Some(relative_thumb) = &asset.thumb_path {
            let thumb = paths::from_rel_string(&snapshot.root, relative_thumb);
            if let Some(source) = safe_source(&canonical_root, &thumb)? {
                let relative = format!("media/thumbs/{id}.webp");
                copy(&source, &destination.join(&relative))?;
                exported.thumb = Some(relative);
            }
        }
        out.insert(id, exported);
    }
    Ok((out, copied, missing))
}

fn safe_source(canonical_root: &Path, source: &Path) -> Result<Option<PathBuf>> {
    let canonical = match fs::canonicalize(source) {
        Ok(path) => path,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(error) => return Err(Error::io(source, error)),
    };
    if !canonical.starts_with(canonical_root) {
        return Err(Error::Malformed {
            path: source.to_path_buf(),
            reason: "asset resolves outside the project folder".to_string(),
        });
    }
    if !canonical.is_file() {
        return Ok(None);
    }
    Ok(Some(canonical))
}

fn wanted_assets(snapshot: &WikiSnapshot) -> BTreeSet<Id> {
    let mut ids = BTreeSet::new();
    for node in &snapshot.nodes {
        ids.extend(node.cover_asset_id);
        ids.extend(node.asset_links.iter().map(|link| link.asset_id));
    }
    for generation in &snapshot.generations {
        ids.extend(generation.output_asset_ids.iter().copied());
    }
    ids
}

fn copy(source: &Path, destination: &Path) -> Result<()> {
    fs::copy(source, destination).map(|_| ()).map_err(|error| Error::io(destination, error))
}

fn write(path: &Path, contents: &[u8]) -> Result<()> {
    fs::write(path, contents).map_err(|error| Error::io(path, error))
}

fn index_page(snapshot: &WikiSnapshot, media: &HashMap<Id, ExportMedia>) -> String {
    let mut body = format!(
        "<section class=\"hero\"><p class=\"eyebrow\">Static world wiki</p><h1>{}</h1>\
         <p>{} nodes · {} generation records</p></section>",
        text(&snapshot.project_name),
        snapshot.nodes.len(),
        snapshot.generations.len()
    );
    for definition in wobu_core::kind_registry() {
        let mut nodes: Vec<_> =
            snapshot.nodes.iter().filter(|node| node.kind == definition.kind).collect();
        if nodes.is_empty() {
            continue;
        }
        nodes.sort_by_key(|node| node.name.to_lowercase());
        body.push_str(&format!(
            "<section><h2>{}</h2><div class=\"cards\">",
            text(definition.plural)
        ));
        for node in nodes {
            body.push_str("<article class=\"card\">");
            if let Some(id) = node.cover_asset_id {
                body.push_str(&media_image(media.get(&id), "", &node.name));
            }
            body.push_str(&format!(
                "<div><p class=\"eyebrow\">{}</p><h3><a href=\"nodes/{}.html\">{}</a></h3><p>{}</p>{}</div>",
                text(definition.label),
                id_attr(node.id),
                text(&node.name),
                text(if node.summary.trim().is_empty() { "No summary yet." } else { &node.summary }),
                tags(&node.tags)
            ));
            body.push_str("</article>");
        }
        body.push_str("</div></section>");
    }
    page(snapshot, &snapshot.project_name, "", &body)
}

fn node_page(snapshot: &WikiSnapshot, node: &Node, media: &HashMap<Id, ExportMedia>) -> String {
    let definition = kind_def(node.kind);
    let mut body = format!(
        "<article class=\"node-page\"><header class=\"node-head\"><p class=\"eyebrow\">{}</p><h1>{}</h1><p class=\"lede\">{}</p>{}</header>",
        text(definition.label),
        text(&node.name),
        text(if node.summary.trim().is_empty() { "No summary yet." } else { &node.summary }),
        tags(&node.tags)
    );
    if let Some(cover) = node.cover_asset_id {
        body.push_str("<section><h2>Cover</h2><div class=\"gallery\">");
        body.push_str(&media_figure(media.get(&cover), "../", &node.name, "Cover"));
        body.push_str("</div></section>");
    }
    if !node.attributes.is_empty() {
        body.push_str("<section><h2>Facts</h2><dl class=\"facts\">");
        for (key, value) in &node.attributes {
            body.push_str(&format!(
                "<div><dt>{}</dt><dd>{}</dd></div>",
                text(&title_case(key)),
                text(&value_text(value))
            ));
        }
        body.push_str("</dl></section>");
    }
    if !node.notes_raw.trim().is_empty() {
        body.push_str("<section><h2>Notes</h2><div class=\"prose\">");
        body.push_str(&render_markdown(&node.notes_raw));
        body.push_str("</div></section>");
    }
    if let Some(description) = &node.description
        && !description.is_empty()
    {
        body.push_str("<section><h2>Description</h2><div class=\"description\">");
        for (key, value) in &description.sections {
            let label = definition
                .sections
                .iter()
                .find(|section| section.key == key)
                .map_or_else(|| title_case(key), |section| section.label.to_string());
            body.push_str(&format!("<section><h3>{}</h3>", text(&label)));
            match value {
                SectionValue::Text(value) => body.push_str(&render_markdown(value)),
                SectionValue::List(values) => {
                    body.push_str("<ul>");
                    for value in values {
                        body.push_str(&format!("<li>{}</li>", text(value)));
                    }
                    body.push_str("</ul>");
                }
            }
            body.push_str("</section>");
        }
        body.push_str("</div></section>");
    }
    body.push_str(&relations(snapshot, node));
    body.push_str(&references(node, media));
    body.push_str(&concepts(snapshot, node, media));
    body.push_str("</article>");
    page(snapshot, &node.name, "../", &body)
}

fn relations(snapshot: &WikiSnapshot, node: &Node) -> String {
    let by_id: HashMap<Id, &Node> = snapshot.nodes.iter().map(|item| (item.id, item)).collect();
    let mut rows = Vec::new();
    if let Some(parent) = node.parent_id.and_then(|id| by_id.get(&id)) {
        rows.push(format!(
            "<li><span>Nested under</span><a href=\"{}.html\">{}</a></li>",
            id_attr(parent.id),
            text(&parent.name)
        ));
    }
    for link in &node.links {
        if let Some(target) = by_id.get(&link.to_id) {
            rows.push(format!(
                "<li{}><span>{} · {:.2}</span><a href=\"{}.html\">{}</a></li>",
                if link.enabled { "" } else { " class=\"muted\"" },
                text(link.role.label()),
                link.weight,
                id_attr(target.id),
                text(&target.name)
            ));
        }
    }
    for source in &snapshot.nodes {
        if source.parent_id == Some(node.id) {
            rows.push(format!(
                "<li><span>Contains</span><a href=\"{}.html\">{}</a></li>",
                id_attr(source.id),
                text(&source.name)
            ));
        }
        for link in source.links.iter().filter(|link| link.to_id == node.id) {
            rows.push(format!(
                "<li{}><span>{} from</span><a href=\"{}.html\">{}</a></li>",
                if link.enabled { "" } else { " class=\"muted\"" },
                text(link.role.label()),
                id_attr(source.id),
                text(&source.name)
            ));
        }
    }
    if rows.is_empty() {
        return String::new();
    }
    format!("<section><h2>Relations</h2><ul class=\"relations\">{}</ul></section>", rows.join(""))
}

fn references(node: &Node, media: &HashMap<Id, ExportMedia>) -> String {
    if node.asset_links.is_empty() {
        return String::new();
    }
    let mut out = String::from("<section><h2>References</h2><div class=\"gallery\">");
    for link in &node.asset_links {
        let muted = if link.enabled { "" } else { " · muted" };
        out.push_str(&media_figure(
            media.get(&link.asset_id),
            "../",
            &format!("{} reference", node.name),
            &format!("{} · {:.2}{muted}", link.role.label(), link.weight),
        ));
    }
    out.push_str("</div></section>");
    out
}

fn concepts(snapshot: &WikiSnapshot, node: &Node, media: &HashMap<Id, ExportMedia>) -> String {
    let generations: Vec<_> = snapshot
        .generations
        .iter()
        .filter(|generation| {
            generation.node_id == node.id && !generation.output_asset_ids.is_empty()
        })
        .collect();
    if generations.is_empty() {
        return String::new();
    }
    let mut out = String::from("<section><h2>Concepts</h2><div class=\"gallery\">");
    for generation in generations {
        let date = generation.created_at.format("%Y-%m-%d");
        for id in &generation.output_asset_ids {
            out.push_str(&media_figure(
                media.get(id),
                "../",
                &format!("{} concept", node.name),
                &format!(
                    "{} · {} · {} · seed {}",
                    generation.preset, date, generation.model, generation.seed
                ),
            ));
        }
    }
    out.push_str("</div></section>");
    out
}

fn media_figure(media: Option<&ExportMedia>, prefix: &str, alt: &str, caption: &str) -> String {
    match media.and_then(|item| item.original.as_ref().map(|original| (item, original))) {
        Some((item, original)) => {
            let source = item.thumb.as_ref().unwrap_or(original);
            format!(
                "<figure><a href=\"{}{}\"><img src=\"{}{}\" alt=\"{}\" loading=\"lazy\"></a><figcaption>{}</figcaption></figure>",
                attr(prefix),
                attr(original),
                attr(prefix),
                attr(source),
                attr(alt),
                text(caption)
            )
        }
        None => format!(
            "<figure class=\"missing\"><div>Image unavailable</div><figcaption>{}</figcaption></figure>",
            text(caption)
        ),
    }
}

fn media_image(media: Option<&ExportMedia>, prefix: &str, alt: &str) -> String {
    let Some(item) = media else { return String::new() };
    let Some(source) = item.thumb.as_ref().or(item.original.as_ref()) else { return String::new() };
    format!(
        "<img class=\"cover\" src=\"{}{}\" alt=\"{}\" loading=\"lazy\">",
        attr(prefix),
        attr(source),
        attr(alt)
    )
}

fn graph_page(snapshot: &WikiSnapshot) -> String {
    let columns = 4usize;
    let card_width = 190usize;
    let card_height = 56usize;
    let gap_x = 38usize;
    let gap_y = 44usize;
    let positions: HashMap<Id, (usize, usize)> = snapshot
        .nodes
        .iter()
        .enumerate()
        .map(|(index, node)| {
            (
                node.id,
                (
                    40 + (index % columns) * (card_width + gap_x),
                    60 + (index / columns) * (card_height + gap_y),
                ),
            )
        })
        .collect();
    let width = 40 + columns * (card_width + gap_x);
    let rows = snapshot.nodes.len().div_ceil(columns).max(1);
    let height = 70 + rows * (card_height + gap_y);
    let mut svg = format!(
        "<svg class=\"world-graph\" viewBox=\"0 0 {width} {height}\" role=\"img\" aria-label=\"{} nodes and their influence relationships\"><defs><marker id=\"arrow\" viewBox=\"0 0 8 8\" refX=\"7\" refY=\"4\" markerWidth=\"6\" markerHeight=\"6\" orient=\"auto\"><path d=\"M0 0L8 4L0 8Z\"></path></marker></defs>",
        snapshot.nodes.len()
    );
    for node in &snapshot.nodes {
        let Some(&(x1, y1)) = positions.get(&node.id) else { continue };
        if let Some(parent) = node.parent_id.and_then(|id| positions.get(&id)) {
            svg.push_str(&edge(x1, y1, *parent, card_width, card_height, "parent", "Nested under"));
        }
        for link in &node.links {
            if let Some(target) = positions.get(&link.to_id) {
                let class = if link.enabled { "influence" } else { "influence muted" };
                svg.push_str(&edge(
                    x1,
                    y1,
                    *target,
                    card_width,
                    card_height,
                    class,
                    link.role.label(),
                ));
            }
        }
    }
    for node in &snapshot.nodes {
        let Some(&(x, y)) = positions.get(&node.id) else { continue };
        let definition = kind_def(node.kind);
        svg.push_str(&format!(
            "<a href=\"nodes/{}.html\"><g class=\"graph-node\"><rect x=\"{x}\" y=\"{y}\" width=\"{card_width}\" height=\"{card_height}\" rx=\"8\" style=\"--kind:{}\"></rect><text x=\"{}\" y=\"{}\"><tspan>{}</tspan><tspan x=\"{}\" dy=\"18\" class=\"kind\">{}</tspan></text></g></a>",
            id_attr(node.id),
            attr(definition.color),
            x + 12,
            y + 22,
            text(&truncate(&node.name, 25)),
            x + 12,
            text(definition.label)
        ));
    }
    svg.push_str("</svg>");
    let body = format!(
        "<section class=\"hero\"><p class=\"eyebrow\">Project map</p><h1>Influence graph</h1><p>Solid arrows are explicit influence links; dashed arrows are nesting.</p></section><div class=\"graph-wrap\">{svg}</div>"
    );
    page(snapshot, "Influence graph", "", &body)
}

fn edge(
    x1: usize,
    y1: usize,
    target: (usize, usize),
    width: usize,
    height: usize,
    class: &str,
    label: &str,
) -> String {
    let (x2, y2) = target;
    format!(
        "<line class=\"{}\" x1=\"{}\" y1=\"{}\" x2=\"{}\" y2=\"{}\" marker-end=\"url(#arrow)\"><title>{}</title></line>",
        attr(class),
        x1 + width / 2,
        y1 + height / 2,
        x2 + width / 2,
        y2 + height / 2,
        text(label)
    )
}

fn page(snapshot: &WikiSnapshot, title: &str, prefix: &str, body: &str) -> String {
    format!(
        "<!doctype html><html lang=\"en\"><head><meta charset=\"utf-8\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><meta name=\"generator\" content=\"Wobu static world wiki\"><title>{} · {}</title><link rel=\"stylesheet\" href=\"{}site.css\"></head><body><header class=\"site-head\"><a href=\"{}index.html\" class=\"brand\">{}</a><nav><a href=\"{}index.html\">World</a><a href=\"{}graph.html\">Influence graph</a></nav></header><main>{}</main><footer>Exported from Wobu · canonical source remains the project folder.</footer></body></html>",
        text(title),
        text(&snapshot.project_name),
        attr(prefix),
        attr(prefix),
        text(&snapshot.project_name),
        attr(prefix),
        attr(prefix),
        body
    )
}

/// A deliberately small, safe Markdown projection. Wobu notes are free-form;
/// HTML is escaped first and only block syntax we emit ourselves becomes markup.
fn render_markdown(markdown: &str) -> String {
    let mut out = String::new();
    let mut list = false;
    let mut code = false;
    for raw in markdown.replace("\r\n", "\n").lines() {
        let line = raw.trim_end();
        if line.trim_start().starts_with("```") {
            if list {
                out.push_str("</ul>");
                list = false;
            }
            out.push_str(if code { "</code></pre>" } else { "<pre><code>" });
            code = !code;
            continue;
        }
        if code {
            out.push_str(&text(line));
            out.push('\n');
            continue;
        }
        if let Some(item) = line.trim_start().strip_prefix("- ") {
            if !list {
                out.push_str("<ul>");
                list = true;
            }
            out.push_str(&format!("<li>{}</li>", text(item)));
            continue;
        }
        if list {
            out.push_str("</ul>");
            list = false;
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let (tag, value) = if let Some(value) = trimmed.strip_prefix("### ") {
            ("h4", value)
        } else if let Some(value) = trimmed.strip_prefix("## ") {
            ("h3", value)
        } else if let Some(value) = trimmed.strip_prefix("# ") {
            ("h3", value)
        } else if let Some(value) = trimmed.strip_prefix("> ") {
            ("blockquote", value)
        } else {
            ("p", trimmed)
        };
        out.push_str(&format!("<{tag}>{}</{tag}>", text(value)));
    }
    if list {
        out.push_str("</ul>");
    }
    if code {
        out.push_str("</code></pre>");
    }
    out
}

fn tags(values: &[String]) -> String {
    if values.is_empty() {
        return String::new();
    }
    format!(
        "<ul class=\"tags\">{}</ul>",
        values.iter().map(|value| format!("<li>#{}</li>", text(value))).collect::<String>()
    )
}

fn value_text(value: &serde_json::Value) -> String {
    value.as_str().map(str::to_owned).unwrap_or_else(|| value.to_string())
}

fn title_case(value: &str) -> String {
    let mut words = value.split(['_', '-']).filter(|word| !word.is_empty());
    let Some(first) = words.next() else { return String::new() };
    let mut out = capitalise(first);
    for word in words {
        out.push(' ');
        out.push_str(&capitalise(word));
    }
    out
}

fn capitalise(value: &str) -> String {
    let mut chars = value.chars();
    chars.next().map_or_else(String::new, |first| first.to_uppercase().chain(chars).collect())
}

fn truncate(value: &str, max: usize) -> String {
    let mut chars = value.chars();
    let head: String = chars.by_ref().take(max).collect();
    if chars.next().is_some() { format!("{head}…") } else { head }
}

fn id_attr(id: Id) -> String {
    attr(&id.to_string())
}

fn text(value: &str) -> String {
    escape(value, false)
}

fn attr(value: &str) -> String {
    escape(value, true)
}

fn escape(value: &str, attribute: bool) -> String {
    let mut out = String::with_capacity(value.len());
    for ch in value.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' if attribute => out.push_str("&#39;"),
            _ => out.push(ch),
        }
    }
    out
}

const SITE_CSS: &str = r#":root{color-scheme:dark;--bg:#0d0e12;--panel:#15171d;--raised:#1d2028;--line:#303541;--text:#e8eaf0;--dim:#9aa1b3;--faint:#697184;--accent:#e2a44f}*{box-sizing:border-box}body{margin:0;background:var(--bg);color:var(--text);font:15px/1.6 system-ui,-apple-system,sans-serif}.site-head{position:sticky;z-index:10;top:0;display:flex;justify-content:space-between;gap:24px;padding:14px max(22px,calc((100vw - 1120px)/2));border-bottom:1px solid var(--line);background:color-mix(in srgb,var(--panel) 94%,transparent);backdrop-filter:blur(12px)}a{color:var(--accent);text-decoration:none}a:hover{text-decoration:underline}.brand{color:var(--text);font-weight:750}.site-head nav{display:flex;gap:18px}main{width:min(1120px,calc(100% - 36px));margin:0 auto;padding:36px 0 70px}.hero{padding:24px 0 34px}.hero h1,.node-head h1{margin:.1em 0;font-size:clamp(2rem,5vw,4.4rem);line-height:1.05}.hero p,.lede{color:var(--dim)}.eyebrow{margin:0;color:var(--accent);font-size:.72rem;font-weight:700;letter-spacing:.12em;text-transform:uppercase}section>h2{margin:44px 0 16px;padding-bottom:8px;border-bottom:1px solid var(--line)}.cards{display:grid;grid-template-columns:repeat(auto-fill,minmax(260px,1fr));gap:14px}.card{overflow:hidden;border:1px solid var(--line);border-radius:12px;background:var(--panel)}.card>div{padding:16px}.card h3{margin:3px 0}.card p{margin:4px 0;color:var(--dim)}.cover{display:block;width:100%;height:180px;object-fit:cover;background:var(--raised)}.tags{display:flex;flex-wrap:wrap;gap:6px;margin:12px 0 0;padding:0;list-style:none}.tags li{padding:2px 7px;border-radius:99px;background:var(--raised);color:var(--faint);font-size:.72rem}.node-page{max-width:900px}.node-head{padding:24px 0}.facts{display:grid;grid-template-columns:repeat(auto-fit,minmax(200px,1fr));gap:8px}.facts div{padding:10px;border:1px solid var(--line);border-radius:8px}.facts dt{color:var(--faint);font-size:.72rem;text-transform:uppercase}.facts dd{margin:3px 0 0}.prose,.description{color:var(--dim)}.prose h3,.prose h4,.description h3{color:var(--text)}pre{overflow:auto;padding:14px;border-radius:8px;background:#090a0d;color:var(--dim)}blockquote{margin-left:0;padding-left:14px;border-left:3px solid var(--accent)}.gallery{display:grid;grid-template-columns:repeat(auto-fill,minmax(210px,1fr));gap:14px}.gallery figure{margin:0;overflow:hidden;border:1px solid var(--line);border-radius:10px;background:var(--panel)}.gallery img,.gallery .missing div{display:block;width:100%;height:210px;object-fit:contain;background:#090a0d}.gallery .missing div{display:grid;place-items:center;color:var(--faint)}figcaption{padding:9px 11px;color:var(--dim);font-size:.78rem}.relations{padding:0;list-style:none}.relations li{display:flex;gap:12px;padding:8px 0;border-bottom:1px solid var(--line)}.relations span{min-width:130px;color:var(--faint)}.muted{opacity:.48}.graph-wrap{overflow:auto;border:1px solid var(--line);border-radius:12px;background:#090a0d}.world-graph{display:block;min-width:900px;width:100%;height:auto}.world-graph line{stroke:#758098;stroke-width:1.5;marker-end:url(#arrow)}.world-graph line.parent{stroke-dasharray:5 5}.world-graph line.muted{opacity:.25}.world-graph marker path{fill:#758098}.graph-node rect{fill:var(--panel);stroke:var(--kind);stroke-width:2}.graph-node:hover rect{fill:var(--raised);stroke:var(--accent)}.graph-node text{fill:var(--text);font:600 13px system-ui}.graph-node text.kind{fill:var(--faint);font-size:10px;text-transform:uppercase}footer{padding:22px;text-align:center;border-top:1px solid var(--line);color:var(--faint);font-size:.75rem}@media(max-width:650px){.site-head{align-items:flex-start;flex-direction:column;gap:5px}.hero h1,.node-head h1{font-size:2.4rem}.facts{grid-template-columns:1fr}}"#;

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn text_and_attribute_contexts_escape_active_markup() {
        assert_eq!(text("<script>&\"'"), "&lt;script&gt;&amp;&quot;'");
        assert_eq!(attr("<script>&\"'"), "&lt;script&gt;&amp;&quot;&#39;");
    }

    #[test]
    fn markdown_is_a_safe_block_projection_not_an_html_passthrough() {
        let html = render_markdown("# Heading\n- one\n- <img src=x onerror=boom>\n```\n& raw\n```");
        assert!(html.contains("<h3>Heading</h3>"));
        assert!(html.contains("<ul><li>one</li><li>&lt;img src=x onerror=boom&gt;</li></ul>"));
        assert!(html.contains("<pre><code>&amp; raw\n</code></pre>"));
        assert!(!html.contains("<img"));
    }
}
