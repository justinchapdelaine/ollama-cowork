use crate::{
    contract::{Response, SCHEMA_VERSION, SectionSummary},
    filesystem, package,
};
use anyhow::{Context, Result, bail};
use quick_xml::{Reader, events::Event};
use std::{fs, path::Path};

#[derive(Clone)]
struct Paragraph {
    start: usize,
    end: usize,
    text: String,
    style: Option<String>,
}

fn paragraphs(xml: &str) -> Result<Vec<Paragraph>> {
    let mut reader = Reader::from_str(xml);
    let mut result = Vec::new();
    let mut current: Option<(usize, String, Option<String>)> = None;
    loop {
        let event_start = reader.buffer_position() as usize;
        match reader.read_event()? {
            Event::Start(e) if e.name().as_ref() == b"w:p" => {
                current = Some((event_start, String::new(), None))
            }
            Event::Empty(e) if e.name().as_ref() == b"w:pStyle" => {
                if let Some((_, _, style)) = &mut current {
                    for a in e.attributes() {
                        let a = a?;
                        if a.key.as_ref() == b"w:val" {
                            *style = Some(a.unescape_value()?.into_owned());
                        }
                    }
                }
            }
            Event::Text(e) => {
                if let Some((_, text, _)) = &mut current {
                    text.push_str(&e.decode()?);
                }
            }
            Event::End(e) if e.name().as_ref() == b"w:p" => {
                if let Some((start, text, style)) = current.take() {
                    result.push(Paragraph {
                        start,
                        end: reader.buffer_position() as usize,
                        text,
                        style,
                    });
                }
            }
            Event::Eof => break,
            _ => {}
        }
    }
    Ok(result)
}

fn is_heading(p: &Paragraph) -> bool {
    p.style
        .as_deref()
        .is_some_and(|s| s.eq_ignore_ascii_case("Heading1"))
}
fn summaries(xml: &str) -> Result<Vec<SectionSummary>> {
    let ps = paragraphs(xml)?;
    let mut sections = Vec::new();
    for (i, p) in ps.iter().enumerate().filter(|(_, p)| is_heading(p)) {
        let end = ps
            .iter()
            .enumerate()
            .skip(i + 1)
            .find(|(_, p)| is_heading(p))
            .map(|(j, _)| j)
            .unwrap_or(ps.len());
        sections.push(SectionSummary {
            heading: p.text.clone(),
            paragraphs: ps[i + 1..end]
                .iter()
                .map(|p| p.text.clone())
                .filter(|t| !t.is_empty())
                .collect(),
        });
    }
    Ok(sections)
}

fn escape(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}
fn replacement_xml(items: &[String]) -> String {
    items
        .iter()
        .map(|p| {
            format!(
                "<w:p><w:r><w:t xml:space=\"preserve\">{}</w:t></w:r></w:p>",
                escape(p)
            )
        })
        .collect()
}

pub fn inspect(input: &Path) -> Result<Response> {
    let input = filesystem::canonical_input(input)?;
    let hash = filesystem::sha256(&input)?;
    let xml = package::read_document_xml(&input)?;
    Ok(Response::Inspected {
        schema_version: SCHEMA_VERSION,
        source_sha256: hash,
        sections: summaries(&xml)?,
    })
}
pub fn validate(input: &Path) -> Result<Response> {
    let input = filesystem::canonical_input(input)?;
    let hash = filesystem::sha256(&input)?;
    let xml = package::read_document_xml(&input)?;
    let sections = summaries(&xml)?;
    if sections.is_empty() {
        bail!("DOCX contains no supported Heading1 sections")
    }
    Ok(Response::Valid {
        schema_version: SCHEMA_VERSION,
        source_sha256: hash,
        sections,
    })
}
pub fn rewrite_section(
    input: &Path,
    output: &Path,
    heading: &str,
    replacement: &[String],
) -> Result<Response> {
    let input = filesystem::canonical_input(input)?;
    let output = filesystem::validate_new_output(&input, output)?;
    let source_hash = filesystem::sha256(&input)?;
    let xml = package::read_document_xml(&input)?;
    let ps = paragraphs(&xml)?;
    let matches: Vec<_> = ps
        .iter()
        .enumerate()
        .filter(|(_, p)| is_heading(p) && p.text == heading)
        .collect();
    if matches.len() != 1 {
        bail!("heading must match exactly once; found {}", matches.len())
    }
    let (index, _) = matches[0];
    let body_start = ps.get(index + 1).map(|p| p.start).unwrap_or(ps[index].end);
    let next = ps.iter().skip(index + 1).find(|p| is_heading(p));
    let body_end = next
        .map(|p| p.start)
        .unwrap_or_else(|| xml.find("<w:sectPr").unwrap_or(xml.len()));
    let replaced = ps[index + 1..]
        .iter()
        .take_while(|p| p.start < body_end)
        .count();
    let mut updated = String::with_capacity(xml.len());
    updated.push_str(&xml[..body_start]);
    updated.push_str(&replacement_xml(replacement));
    updated.push_str(&xml[body_end..]);
    let temp = output.with_extension(format!(
        "{}.tmp",
        output
            .extension()
            .and_then(|v| v.to_str())
            .unwrap_or("docx")
    ));
    if temp.exists() {
        fs::remove_file(&temp)?
    }
    package::copy_with_document_xml(&input, &temp, &updated)?;
    package::read_document_xml(&temp).context("validate revised DOCX")?;
    if filesystem::sha256(&input)? != source_hash {
        let _ = fs::remove_file(&temp);
        bail!("source changed during rewrite")
    }
    fs::rename(&temp, &output).context("publish revised copy")?;
    let output_hash = filesystem::sha256(&output)?;
    Ok(Response::Rewritten {
        schema_version: SCHEMA_VERSION,
        source_sha256: source_hash,
        output_sha256: output_hash,
        output,
        heading: heading.into(),
        replaced_paragraph_count: replaced,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn escapes_xml_text() {
        assert_eq!(escape("<&>\"'"), "&lt;&amp;&gt;&quot;&apos;");
    }
    #[test]
    fn finds_heading_sections() {
        let xml = r#"<w:document><w:body><w:p><w:pPr><w:pStyle w:val="Heading1"/></w:pPr><w:r><w:t>One</w:t></w:r></w:p><w:p><w:r><w:t>Body</w:t></w:r></w:p></w:body></w:document>"#;
        let sections = summaries(xml).unwrap();
        assert_eq!(sections.len(), 1);
        assert_eq!(sections[0].heading, "One");
        assert_eq!(sections[0].paragraphs, vec!["Body"]);
    }
}
