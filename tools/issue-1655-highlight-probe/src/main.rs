use tree_sitter_highlight::{HighlightConfiguration, Highlighter, HtmlRenderer};

const CAPTURES: &[&str] = &[
    "comment",
    "keyword",
    "string",
    "number",
    "function",
    "function.builtin",
    "type",
    "type.builtin",
    "variable",
    "variable.builtin",
    "constant",
    "operator",
    "punctuation",
    "constructor",
    "module",
];

const ELISP: &str = ";; Better completing-read implementation\n(use-package consult\n  :after\n  (project)\n  :bind\n  (([remap goto-line] . consult-goto-line)\n   ([remap switch-to-buffer] . consult-buffer)\n   ([remap yank-pop] . consult-yank-pop))\n  :config\n  (advice-add #'project-find-regexp :override #'consult-ripgrep))\n";
// https://tendentious.org/~mdorman/2013/08/26/using-user-authentication-with-couchdb-and-couchdb-conduit
const HASKELL: &str = r#"data UserCredentials = UserCredentials {
    credentialEmail :: ByteString, -- ^The email address of the new user
    credentialPassword :: ByteString  -- ^The password for the new user
} deriving (Show)

connection :: CouchConnection
connection = def {couchLogin = "administrator", couchPass = "ThisIsn'tReallyThePassword"}

userDb :: ByteString -> ByteString
userDb = (intercalate "/") . reverse . splitWith (`elem` "@.")

authId :: ByteString -> ByteString
authId email = concat ["org.couchdb.user:", email]

authRecord :: AntilibrationCredentials -> Value
authRecord (AntilibrationCredentials email password) = object ["name" .= email, "roles" .= ([] :: [ByteString]), "type" .= ("user" :: ByteString), "password" .= password]

createUserDB :: UserCredentials -> IO ()
createUserDB credentials =
  runCouch connection $ do
    _ <- couchPut "_users" (authId $ credentialEmail credentials) "" [] (authRecord credentials)
    couchPutDB_ (userDb $ credentialEmail credentials)
    couchSecureDB (userDb $ credentialEmail credentials) [] [] [] [(credentialEmail credentials)]
"#;

fn probe(
    name: &str,
    code: &str,
    mut config: HighlightConfiguration,
    show: bool,
) -> Result<(), Box<dyn std::error::Error>> {
    config.configure(CAPTURES);
    let mut highlighter = Highlighter::new();
    let events = highlighter.highlight(&config, code.as_bytes(), None, None, |_| None)?;
    let mut html = HtmlRenderer::new();
    html.render(events, code.as_bytes(), &|capture, attrs| {
        let category = match CAPTURES[capture.0].split('.').next().unwrap_or("unknown") {
            "module" | "constructor" => "type",
            other => other,
        };
        attrs.extend_from_slice(format!("class=\"j-syn-{category}\"").as_bytes());
    })?;
    let mut output = html.lines().collect::<String>();
    // HtmlRenderer::lines() always terminates its last line, even when source does not.
    if !code.ends_with('\n') && output.ends_with('\n') {
        output.pop();
    }
    let mut bare = String::new();
    let mut remaining = output.as_str();
    while let Some((text, tag)) = remaining.split_once('<') {
        bare.push_str(text);
        let (tag, after) = tag.split_once('>').ok_or("unterminated tag")?;
        if !tag.starts_with("span ") && tag != "/span" {
            return Err(format!("unexpected HTML tag: {tag}").into());
        }
        remaining = after;
    }
    bare.push_str(remaining);
    let decoded = html_escape::decode_html_entities(&bare);
    if decoded != code {
        return Err(format!("{name}: highlighted text changed: expected {} bytes / got {} bytes; expected tail {:?}; got tail {:?}", code.len(), decoded.len(), code.chars().rev().take(48).collect::<String>(), decoded.chars().rev().take(48).collect::<String>()).into());
    }
    if show {
        println!("{name}:\n{output}");
    } else {
        println!("{name}: {} UTF-8 bytes preserved", code.len());
    }
    Ok(())
}

// Reproduce the two current production exporters without changing the host.
// The oracle is their decoded <code> text, not the authored input, because
// exporters can change a terminal newline even before highlighting.
fn exported_code(
    format: &str,
    language: &str,
    code: &str,
) -> Result<String, Box<dyn std::error::Error>> {
    let body = match format {
        "Org" => format!("#+begin_src {language}\n{code}\n#+end_src\n"),
        "Markdown" => format!("```{language}\n{code}\n```\n"),
        _ => return Err("unknown format".into()),
    };
    let html = if format == "Org" {
        orgize::Org::parse(&body).to_html()
    } else {
        let mut options = pulldown_cmark::Options::empty();
        options.insert(pulldown_cmark::Options::ENABLE_STRIKETHROUGH);
        options.insert(pulldown_cmark::Options::ENABLE_TABLES);
        options.insert(pulldown_cmark::Options::ENABLE_FOOTNOTES);
        options.insert(pulldown_cmark::Options::ENABLE_TASKLISTS);
        let mut html = String::new();
        pulldown_cmark::html::push_html(&mut html, pulldown_cmark::Parser::new_ext(&body, options));
        html
    };
    let (_, after_code) = html.split_once("<code").ok_or("missing code element")?;
    let (_, escaped) = after_code.split_once('>').ok_or("missing code tag end")?;
    let (escaped, _) = escaped
        .split_once("</code>")
        .ok_or("missing code tag close")?;
    Ok(html_escape::decode_html_entities(escaped).into_owned())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    probe(
        "Emacs Lisp",
        ELISP,
        HighlightConfiguration::new(
            tree_sitter_elisp::LANGUAGE.into(),
            "elisp",
            tree_sitter_elisp::HIGHLIGHTS_QUERY,
            "",
            "",
        )?,
        true,
    )?;
    // Upstream's final `(variable) @type` overrides every function/value
    // classification, even plain references. Keep actual `(name) @type`.
    let haskell_query = tree_sitter_haskell::HIGHLIGHTS_QUERY.replace("\n(variable) @type\n", "\n");
    probe(
        "Haskell",
        HASKELL,
        HighlightConfiguration::new(
            tree_sitter_haskell::LANGUAGE.into(),
            "haskell",
            &haskell_query,
            tree_sitter_haskell::INJECTIONS_QUERY,
            tree_sitter_haskell::LOCALS_QUERY,
        )?,
        true,
    )?;
    for (format, language, code) in [
        ("Org", "emacs-lisp", ELISP),
        ("Markdown", "elisp", ELISP),
        ("Org", "haskell", HASKELL),
        ("Markdown", "hs", HASKELL),
    ] {
        let exported = exported_code(format, language, code)?;
        let config = if language == "emacs-lisp" || language == "elisp" {
            HighlightConfiguration::new(
                tree_sitter_elisp::LANGUAGE.into(),
                "elisp",
                tree_sitter_elisp::HIGHLIGHTS_QUERY,
                "",
                "",
            )?
        } else {
            HighlightConfiguration::new(
                tree_sitter_haskell::LANGUAGE.into(),
                "haskell",
                &haskell_query,
                tree_sitter_haskell::INJECTIONS_QUERY,
                tree_sitter_haskell::LOCALS_QUERY,
            )?
        };
        probe(
            &format!("{format}/{language}/real exporter text"),
            &exported,
            config,
            false,
        )?;
    }
    for (name, code) in [
        (
            "adversarial",
            "<script>alert('x')</script> & \"quote\" é 漢字\n{{< youtube abc >}}\n".to_owned(),
        ),
        (
            "malformed",
            "(() [unterminated \"string\n<!-- raw -->\n".to_owned(),
        ),
        // Both exporters add one terminal LF, so 65,535 source bytes yield
        // exactly 65,536 bytes of decoded <code> text at the limit.
        (
            "64 KiB",
            "(message \"<> & hello\")\n"
                .repeat(4000)
                .get(..65535)
                .ok_or("short fixture")?
                .to_owned(),
        ),
    ] {
        for format in ["Org", "Markdown"] {
            for language in ["elisp", "haskell"] {
                let exported = exported_code(format, language, &code)?;
                let config = if language == "elisp" {
                    HighlightConfiguration::new(
                        tree_sitter_elisp::LANGUAGE.into(),
                        "elisp",
                        tree_sitter_elisp::HIGHLIGHTS_QUERY,
                        "",
                        "",
                    )?
                } else {
                    HighlightConfiguration::new(
                        tree_sitter_haskell::LANGUAGE.into(),
                        "haskell",
                        &haskell_query,
                        tree_sitter_haskell::INJECTIONS_QUERY,
                        tree_sitter_haskell::LOCALS_QUERY,
                    )?
                };
                probe(
                    &format!("{format}/{language}/{name}"),
                    &exported,
                    config,
                    false,
                )?;
            }
        }
    }
    Ok(())
}
