//! Repeatable paired benchmark for source-block rendering (issue #1655).
//! Run on the baseline and on the candidate with the same pinned toolchain.

use common::{post_body::PostBody, render::PostFormat};
use std::{env, error::Error, hint::black_box, process::Command, time::Instant};

fn fixture(format: PostFormat, language: &str, size: usize) -> Result<PostBody, Box<dyn Error>> {
    let (label, line) = match language {
        "elisp" => ("elisp", "(message \"hello <world> & friends\")\n"),
        "haskell" => (
            "haskell",
            "greet name = putStrLn (\"hello <\" ++ name ++ \">\")\n",
        ),
        other => return Err(format!("unknown fixture language: {other}").into()),
    };
    let mut source = String::new();
    while source.len() < size {
        source.push_str(line);
    }
    // Both exporters append one LF to decoded <code> text. Measure the
    // advertised decoded payload size, not the authored source byte count.
    source.truncate(size - 1);
    let body = match format {
        PostFormat::Org => format!("#+begin_src {label}\n{source}\n#+end_src\n"),
        PostFormat::Markdown => format!("```{label}\n{source}\n```\n"),
        PostFormat::Html => return Err("HTML has no eligible blocks".into()),
    };
    Ok(body.parse()?)
}

fn render_once(format: PostFormat, language: &str, size: usize) -> Result<u128, Box<dyn Error>> {
    let body = fixture(format, language, size)?;
    let start = Instant::now();
    black_box(host::render::render(black_box(&body), black_box(&format))?);
    Ok(start.elapsed().as_nanos())
}

fn main() -> Result<(), Box<dyn Error>> {
    let args: Vec<String> = env::args().collect();
    if args.get(1).is_some_and(|arg| arg == "--once") {
        let format = match args.get(2).map(String::as_str) {
            Some("org") => PostFormat::Org,
            Some("markdown") => PostFormat::Markdown,
            _ => return Err("expected org|markdown".into()),
        };
        let language = args.get(3).ok_or("missing language")?;
        let size: usize = args.get(4).ok_or("missing byte size")?.parse()?;
        println!("{}", render_once(format, language, size)?);
        return Ok(());
    }
    let exe = env::current_exe()?;
    for format_label in ["org", "markdown"] {
        let format = if format_label == "org" {
            PostFormat::Org
        } else {
            PostFormat::Markdown
        };
        for language in ["elisp", "haskell"] {
            for size in [1024, 8192, 65536] {
                let mut cold = Vec::new();
                for _ in 0..30 {
                    let output = Command::new(&exe)
                        .args(["--once", format_label, language, &size.to_string()])
                        .output()?;
                    if !output.status.success() {
                        return Err(format!(
                            "cold sample failed: {}",
                            String::from_utf8_lossy(&output.stderr)
                        )
                        .into());
                    }
                    cold.push(String::from_utf8(output.stdout)?.trim().parse::<u128>()?);
                }
                let mut warm = Vec::new();
                for _ in 0..100 {
                    warm.push(render_once(format, language, size)?);
                }
                println!(
                    "{}",
                    serde_json::json!({"format":format_label,"language":language,"size":size,"cold_ns":cold,"warm_ns":warm})
                );
            }
        }
    }
    Ok(())
}
