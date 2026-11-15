//! The language-server prims (docs/editor-crate-plan.md, E7): one call each
//! into `crate::lsp`, about the active buffer of editor pane ID — its file,
//! its language, its text as it stands and the point — so the server is
//! asked about what is on screen, not what was last drawn.

/// The active buffer of pane `id`: (path, language, text, line, byte), or
/// None when it visits no file.
fn at(id: f64) -> Option<(String, String, String, usize, usize)> {
    let id = id as u32;
    let buf = crate::editorpane::active_buffer(id);
    let path = crate::editorpane::buffer_path(id, buf)?;
    let lang = crate::editorpane::buffer_language(id, buf)?;
    let (line, byte) = crate::editorpane::cursor(id);
    Some((path, lang.to_string(), crate::editorpane::text(id), line, byte))
}

prim! {
    /// `(%lsp-register language program markers)` — serve LANGUAGE's
    /// buffers with PROGRAM (a language server on the PATH), one server per
    /// project, a project's root marked by one of MARKERS ("Cargo.toml").
    /// An empty PROGRAM turns it off. Registering again stops what runs.
    "%lsp-register", fn lsp_register(language: text, program: text, markers: textv) {
        crate::lsp::register(&language, &program, &[], &markers);
        Out::Nil
    }
}

prim! {
    /// `(%lsp-complete id)` — what completes at the point in pane ID, as
    /// words, from its language server; #f when there is none to ask or it
    /// did not answer in time.
    "%lsp-complete", fn lsp_complete(id: num) {
        let Some((path, lang, text, line, byte)) = at(id) else { return Out::Nil };
        match crate::lsp::complete(&path, &lang, &text, line, byte) {
            Some(words) => Out::rows(words.into_iter().map(Out::Text)),
            None => Out::Nil,
        }
    }
}

prim! {
    /// `(%lsp-definition id)` — where the name at the point is defined, as
    /// ((path line column "definition") …), line from 0 and column in
    /// characters; #f when there is no server to ask.
    "%lsp-definition", fn lsp_definition(id: num) {
        let Some((path, lang, text, line, byte)) = at(id) else { return Out::Nil };
        match crate::lsp::definition(&path, &lang, &text, line, byte) {
            Some(sites) => Out::rows(sites.into_iter().map(|(p, l, c)| {
                Out::Many(vec![Out::Text(p), Out::Num(l as f64), Out::Num(c as f64), Out::Text("definition".into())])
            })),
            None => Out::Nil,
        }
    }
}

prim! {
    /// `(%lsp-hover id)` — (signature text) for the name at the point, or
    /// #f.
    "%lsp-hover", fn lsp_hover(id: num) {
        let Some((path, lang, text, line, byte)) = at(id) else { return Out::Nil };
        match crate::lsp::hover(&path, &lang, &text, line, byte) {
            Some((sig, doc)) => Out::Many(vec![Out::Text(sig), Out::Text(doc)]),
            None => Out::Nil,
        }
    }
}

prim! {
    /// `(%lsp-ask-hint id)` — ask for the argument hint at the point
    /// without waiting; it arrives through `%lsp-take-hint` once the host
    /// is woken. 1 when asked.
    "%lsp-ask-hint", fn lsp_ask_hint(id: num) {
        let Some((path, lang, text, line, byte)) = at(id) else { return Out::Num(0.0) };
        Out::Num(if crate::lsp::ask_hint(&path, &lang, &text, line, byte) { 1.0 } else { 0.0 })
    }
}

prim! {
    /// `(%lsp-take-hint)` — the argument hint that has come back, once; #f.
    "%lsp-take-hint", fn lsp_take_hint() {
        crate::lsp::take_hint().map_or(Out::Nil, Out::Text)
    }
}

prim! {
    /// `(%lsp-saved path)` — PATH was written: its server checks it.
    "%lsp-saved", fn lsp_saved(path: text) {
        crate::lsp::saved(&path);
        Out::Nil
    }
}

prim! {
    /// `(%lsp-status id)` — how pane ID's language server is, in a line.
    "%lsp-status", fn lsp_status(id: num) {
        let Some((path, lang, ..)) = at(id) else { return Out::Text("this buffer visits no file".into()) };
        Out::Text(crate::lsp::status(&path, &lang))
    }
}

prim! {
    /// `(%lsp-stop language)` — stop LANGUAGE's servers, or every server
    /// for #f. The next buffer that needs one starts it again.
    "%lsp-stop", fn lsp_stop(language: opt_text) {
        crate::lsp::stop(language.as_deref());
        Out::Nil
    }
}

pub(super) fn install(interp: &mut scm::Interp) -> Result<Vec<&'static str>, scm::EmbedError> {
    Ok(vec![
        lsp_register::install(interp)?,
        lsp_complete::install(interp)?,
        lsp_definition::install(interp)?,
        lsp_hover::install(interp)?,
        lsp_ask_hint::install(interp)?,
        lsp_take_hint::install(interp)?,
        lsp_saved::install(interp)?,
        lsp_status::install(interp)?,
        lsp_stop::install(interp)?,
    ])
}
