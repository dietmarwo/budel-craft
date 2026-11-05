use crate::Result;
use crate::geometry::Point;
use crate::model::{Answer, Case, Holes};
use serde::Serialize;
use std::fs::{self, File, OpenOptions};
use std::io::{BufWriter, Write};
use std::path::Path;
use std::sync::atomic::{AtomicU64, Ordering};

static NEXT_TEMP: AtomicU64 = AtomicU64::new(0);

pub fn atomic_write(
    path: &Path,
    write: impl FnOnce(&mut BufWriter<File>) -> Result<()>,
) -> Result<()> {
    let parent = path
        .parent()
        .filter(|p| !p.as_os_str().is_empty())
        .unwrap_or(Path::new("."));
    fs::create_dir_all(parent)?;
    let file_name = path
        .file_name()
        .ok_or("output must name a file")?
        .to_string_lossy();
    let tmp = parent.join(format!(
        ".{file_name}.{}.{}.tmp",
        std::process::id(),
        NEXT_TEMP.fetch_add(1, Ordering::Relaxed)
    ));
    let result = (|| {
        let mut writer =
            BufWriter::new(OpenOptions::new().create_new(true).write(true).open(&tmp)?);
        write(&mut writer)?;
        writer.flush()?;
        drop(writer);
        fs::rename(&tmp, path)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = fs::remove_file(&tmp);
    }
    result
}

pub fn json(path: &Path, value: &impl Serialize) -> Result<()> {
    atomic_write(path, |out| {
        serde_json::to_writer_pretty(&mut *out, value)?;
        writeln!(out)?;
        Ok(())
    })
}

fn escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

fn path_data(out: &mut impl Write, points: &[Point]) -> std::io::Result<()> {
    for (i, p) in points.iter().enumerate() {
        write!(
            out,
            "{} {:.7} {:.7} ",
            if i == 0 { "M" } else { "L" },
            p.0,
            p.1
        )?;
    }
    write!(out, "Z ")
}

pub fn svg(case: &Case, answer: &Answer, path: &Path) -> Result<()> {
    if answer.case != case.name {
        return Err("answer case does not match".into());
    }
    atomic_write(path, |out| {
        writeln!(
            out,
            r#"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 {} {}">"#,
            case.plane.w, case.plane.h
        )?;
        writeln!(out, "<title>{}</title>", escape(&case.name))?;
        writeln!(
            out,
            r##"<rect width="100%" height="100%" fill="#f4eedf"/>"##
        )?;
        let Point(w, h) = case.usable.size();
        writeln!(
            out,
            r##"<rect x="{}" y="{}" width="{}" height="{}" fill="none" stroke="#aa5146"/>"##,
            case.usable.min.0, case.usable.min.1, w, h
        )?;
        for pl in &answer.placements {
            let idx = *case
                .ids
                .get(&pl.id)
                .ok_or_else(|| format!("unknown shape {}", pl.id))?;
            if !case.allowed(pl.pose) {
                return Err(format!("forbidden pose for {}", pl.id).into());
            }
            let sh = &case.items[idx];
            writeln!(out, "<g><title>{}</title>", escape(&pl.id))?;
            write!(out, r##"<path fill="#26363d" fill-rule="evenodd" d=""##)?;
            path_data(out, &pl.pose.apply(&sh.artwork))?;
            if case.rules.holes_are == Holes::Paper {
                for hole in &sh.holes {
                    path_data(out, &pl.pose.apply(hole))?;
                }
            }
            writeln!(out, r#""/>"#)?;
            if case.rules.holes_are == Holes::Ink {
                for hole in &sh.holes {
                    write!(out, r##"<path fill="#aa654e" d=""##)?;
                    path_data(out, &pl.pose.apply(hole))?;
                    writeln!(out, r#""/>"#)?;
                }
            }
            writeln!(out, "</g>")?;
        }
        writeln!(out, "</svg>")?;
        Ok(())
    })
}
