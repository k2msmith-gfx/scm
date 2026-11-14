//! **Outside the binding, no file in `edit` names the VM** — the modeler's
//! rule (`modeler/src/layering.rs`), for the same reason: the widget and the
//! languages are what a second extension language would share, so they
//! must not depend on this one. The binding is `scm_binding.rs` and
//! `scm_binding/` (E2), and nothing else.

#[test]
fn no_file_names_the_vm() {
    let root = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/src"));
    let mut files = Vec::new();
    walk(root, &mut files);
    let mut offenders = Vec::new();
    for path in files {
        let rel = path.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/");
        if rel == "layering.rs" || rel == "scm_binding.rs" || rel.starts_with("scm_binding/") {
            continue;
        }
        let src = std::fs::read_to_string(&path).expect("read");
        for (n, line) in src.lines().enumerate() {
            let code = line.trim_start();
            if code.starts_with("//") {
                continue;
            }
            if line.contains("scm::") || line.contains("schemers") {
                offenders.push(format!("{rel}:{}: {}", n + 1, line.trim()));
            }
        }
    }
    assert!(offenders.is_empty(), "edit/src names the VM:\n{}", offenders.join("\n"));
}

fn walk(dir: &std::path::Path, out: &mut Vec<std::path::PathBuf>) {
    for entry in std::fs::read_dir(dir).expect("dir") {
        let path = entry.expect("entry").path();
        if path.is_dir() {
            walk(&path, out);
        } else if path.extension().is_some_and(|e| e == "rs") {
            out.push(path);
        }
    }
}
