// SPDX-License-Identifier: Apache-2.0

use std::fs;
use std::path::{Path, PathBuf};

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .expect("workspace root")
        .to_path_buf()
}

fn collect_named_files(root: &Path, names: &[&str], out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(root).expect("read dir") {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            collect_named_files(&path, names, out);
        } else if path
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| names.contains(&name))
        {
            out.push(path);
        }
    }
}

fn collect_rs_files(root: &Path, out: &mut Vec<PathBuf>) {
    for entry in fs::read_dir(root).expect("read dir") {
        let path = entry.expect("dir entry").path();
        if path.is_dir() {
            collect_rs_files(&path, out);
        } else if path.extension().and_then(|ext| ext.to_str()) == Some("rs") {
            out.push(path);
        }
    }
}

#[test]
fn public_probe_common_src_has_no_axiom_specific_surface() {
    let root = workspace_root().join("crates/laplace-probe-common/src");
    let mut files = Vec::new();
    collect_rs_files(&root, &mut files);

    let forbidden = ["AxiomStepBuilder", "MAX_AXIOM_THREADS", "laplace_axiom"];
    let mut violations = Vec::new();
    for file in files {
        let text = fs::read_to_string(&file).expect("read source");
        for term in forbidden {
            if text.contains(term) {
                violations.push(format!("{} contains {term}", file.display()));
            }
        }
    }

    assert!(violations.is_empty(), "{}", violations.join("\n"));
}

#[test]
fn public_manifests_do_not_leak_private_paths_or_verification_feature() {
    let root = workspace_root();
    let mut manifests = Vec::new();
    collect_named_files(&root.join("examples"), &["Cargo.toml"], &mut manifests);
    collect_named_files(&root.join("vendor"), &["Cargo.toml"], &mut manifests);

    let mut violations = Vec::new();
    for manifest in manifests {
        let text = fs::read_to_string(&manifest).expect("read manifest");
        if text.contains("open/crates") {
            violations.push(format!("{} contains open/crates", manifest.display()));
        }
        if text
            .lines()
            .any(|line| line.contains("features") && line.contains("verification"))
        {
            violations.push(format!(
                "{} enables removed verification feature",
                manifest.display()
            ));
        }
        for line in text.lines() {
            let Some((_, raw_path)) = line.split_once("path") else {
                continue;
            };
            let Some((_, quoted)) = raw_path.split_once('"') else {
                continue;
            };
            let Some((relative_path, _)) = quoted.split_once('"') else {
                continue;
            };
            let resolved = manifest
                .parent()
                .expect("manifest parent")
                .join(relative_path);
            if !resolved.exists() {
                violations.push(format!(
                    "{} has unresolved path {}",
                    manifest.display(),
                    relative_path
                ));
            }
        }
    }

    assert!(violations.is_empty(), "{}", violations.join("\n"));
}
