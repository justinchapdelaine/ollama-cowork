use anyhow::{Context, Result, bail};
use std::{
    fs::File,
    io::{Read, Write},
    path::Path,
};
use zip::{ZipArchive, ZipWriter, write::SimpleFileOptions};

const REQUIRED: [&str; 3] = ["[Content_Types].xml", "_rels/.rels", "word/document.xml"];

pub fn read_document_xml(path: &Path) -> Result<String> {
    let file = File::open(path)?;
    let mut zip = ZipArchive::new(file).context("open DOCX ZIP package")?;
    for name in REQUIRED {
        if zip.by_name(name).is_err() {
            bail!("invalid DOCX: missing {name}")
        }
    }
    let mut xml = String::new();
    zip.by_name("word/document.xml")?
        .read_to_string(&mut xml)
        .context("word/document.xml is not UTF-8 XML")?;
    Ok(xml)
}

pub fn copy_with_document_xml(input: &Path, temp_output: &Path, document_xml: &str) -> Result<()> {
    let source = File::open(input)?;
    let mut zip = ZipArchive::new(source)?;
    let target = File::create(temp_output)?;
    let mut writer = ZipWriter::new(target);
    for index in 0..zip.len() {
        let mut entry = zip.by_index(index)?;
        let name = entry.name().to_owned();
        let options = SimpleFileOptions::default().compression_method(entry.compression());
        if entry.is_dir() {
            writer.add_directory(name, options)?;
            continue;
        }
        writer.start_file(name.clone(), options)?;
        if name == "word/document.xml" {
            writer.write_all(document_xml.as_bytes())?;
        } else {
            std::io::copy(&mut entry, &mut writer)?;
        }
    }
    writer.finish()?;
    Ok(())
}
