//! **No file in `edit` names the VM — yet.** The modeler's rule
//! (`modeler/src/layering.rs`): outside the binding, nothing names the VM.
//! E1 has no binding in it, so the rule is absolute; E2 brings the `%editor-*`
//! prims in as `scm_binding.rs`, the one file this will then allow.

#[test]
fn no_file_names_the_vm() {
    let root = std::path::Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/src"));
    let mut files = Vec::new();
    walk(root, &mut files);
    let mut offenders = Vec::new();
    for path in files {
        let rel = path.strip_prefix(root).unwrap().to_string_lossy().replace('\\', "/");
        if rel == "layering.rs" {
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
