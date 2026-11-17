//! Build-only React Compiler orchestration. Node never enters the app's graph.

use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// Project source roots and APIs whose mutable return values must not be memoized.
#[derive(Clone, Debug, Default)]
pub struct ReactCompilerOptions {
    /// Defaults to the nearest package/tsconfig folder above the entry. Explicit
    /// roots may opt dependency distributions into build-time compilation.
    pub roots: Vec<PathBuf>,
    /// Additional `PACKAGE:EXPORT` bailouts, in addition to react-hook-form:useForm.
    pub incompatible_apis: Vec<String>,
    /// Build artifacts only; never included in the standalone app.
    pub cache: Option<PathBuf>,
}

pub(crate) struct Build {
    pub overlay: rsui_js::SourceOverlay,
    originals: Vec<(PathBuf, Vec<u8>)>,
    modules: HashMap<PathBuf, serde_json::Value>,
    helper: PathBuf,
    cache: PathBuf,
}

impl Build {
    pub fn validate(&self) -> Result<(), String> {
        for (path, bytes) in &self.originals {
            if std::fs::read(path).map_err(|error| format!("{}: {error}", path.display()))?
                != *bytes
            {
                return Err(format!(
                    "{} changed while compiling; rebuild the app",
                    path.display()
                ));
            }
        }
        Ok(())
    }
}

pub(crate) fn prepare(entry: &Path, options: &ReactCompilerOptions) -> Result<Build, String> {
    let canonical = |path: &Path| {
        std::fs::canonicalize(path)
            .map(|path| crate::pack::plain_path(&path))
            .map_err(|error| format!("{}: {error}", path.display()))
    };
    let entry = canonical(entry)?;
    let roots = if options.roots.is_empty() {
        let folder = entry.parent().ok_or("entry has no folder")?;
        let root = folder
            .ancestors()
            .find(|folder| {
                ["package.json", "tsconfig.json", "jsconfig.json"]
                    .iter()
                    .any(|name| folder.join(name).is_file())
            })
            .unwrap_or(folder);
        vec![root.to_owned()]
    } else {
        options
            .roots
            .iter()
            .map(|root| canonical(root))
            .collect::<Result<Vec<_>, _>>()?
    };
    if !roots.iter().any(|root| entry.starts_with(root)) {
        return Err("React Compiler entry must be inside a configured source root".to_owned());
    }
    let cache = options
        .cache
        .clone()
        .unwrap_or_else(|| roots[0].join(".rsui/react-compiler"));
    std::fs::create_dir_all(&cache).map_err(|error| format!("compiler cache: {error}"))?;
    let cache = canonical(&cache)?;
    let helper = std::env::var_os("RSUI_REACT_COMPILER_HELPER")
        .map(PathBuf::from)
        .or_else(|| {
            let path = std::env::current_exe()
                .ok()?
                .parent()?
                .join("react-compiler/compile.cjs");
            path.is_file().then_some(path)
        })
        .unwrap_or_else(|| {
            PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("react-compiler/compile.cjs")
        });
    if !helper.is_file() {
        return Err("React Compiler build helper missing; install crates/rsui-tsx/react-compiler and set RSUI_REACT_COMPILER_HELPER to compile.cjs".to_owned());
    }
    let manifest = run_helper(
        &helper,
        &cache,
        serde_json::json!({"entry": entry, "roots": roots, "explicitRoots": !options.roots.is_empty(), "incompatibleApis": options.incompatible_apis}),
    )?;
    let files = manifest["files"]
        .as_array()
        .ok_or("compiler manifest has no files")?;
    let mut sources = Vec::new();
    let mut originals = Vec::new();
    let mut modules = HashMap::new();
    let mut successes = 0;
    let mut skips = 0;
    for file in files {
        let field = |name: &str| {
            file[name]
                .as_str()
                .ok_or_else(|| format!("compiler manifest missing {name}"))
        };
        let input = canonical(Path::new(field("input")?))?;
        if !roots.iter().any(|root| input.starts_with(root))
            || (dependency_path(&input)
                && (options.roots.is_empty()
                    || !roots
                        .iter()
                        .any(|root| dependency_path(root) && input.starts_with(root))))
        {
            return Err(format!(
                "compiler source outside project roots: {}",
                input.display()
            ));
        }
        let original = std::fs::read(field("original")?)
            .map_err(|error| format!("compiler original snapshot: {error}"))?;
        let source = std::fs::read_to_string(field("output")?)
            .map_err(|error| format!("compiler output: {error}"))?;
        for event in file["events"].as_array().into_iter().flatten() {
            if event["kind"] == "CompileSuccess" {
                successes += 1;
            } else if event["kind"] == "CompileSkip" || event["kind"] == "CompileError" {
                skips += 1;
            }
        }
        modules.insert(input.clone(), file.clone());
        originals.push((input.clone(), original));
        sources.push((input, source));
    }
    if !sources.iter().any(|(path, _)| path == &entry) {
        return Err("React Compiler did not produce the entry; use a project .tsx/.jsx/.ts/.js source outside excluded folders".to_owned());
    }
    eprintln!(
        "React Compiler: {} project modules, {successes} compiled functions, {skips} compiler bailouts; diagnostics in {}",
        sources.len(),
        cache.display()
    );
    let build = Build {
        overlay: rsui_js::SourceOverlay::new(sources),
        originals,
        modules,
        helper,
        cache,
    };
    build.validate()?;
    Ok(build)
}

fn run_helper(
    helper: &Path,
    cache: &Path,
    mut json: serde_json::Value,
) -> Result<serde_json::Value, String> {
    let id = format!(
        "{}-{}",
        std::process::id(),
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_nanos()
    );
    let request = cache.join(format!("request-{id}.json"));
    let manifest = cache.join(format!("manifest-{id}.json"));
    json["cache"] = serde_json::json!(cache);
    json["manifest"] = serde_json::json!(manifest);
    std::fs::write(&request, json.to_string())
        .map_err(|error| format!("compiler request: {error}"))?;
    let mut command = std::process::Command::new("node");
    command.arg(helper).arg(&request);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        command.creation_flags(0x08000000);
    }
    let output = command
        .output()
        .map_err(|error| format!("React Compiler requires Node on the build machine: {error}"))?;
    let _ = std::fs::remove_file(request);
    if !output.status.success() {
        return Err(format!(
            "React Compiler failed (install pinned dependencies with npm ci --prefix crates/rsui-tsx/react-compiler):\n{}",
            String::from_utf8_lossy(&output.stderr)
        ));
    }
    let text = std::fs::read_to_string(&manifest)
        .map_err(|error| format!("compiler manifest: {error}"))?;
    serde_json::from_str(&text).map_err(|error| format!("compiler manifest: {error}"))
}

impl Build {
    /// Resolve delayed module edges without running their top levels. Runtime
    /// packages remain uncompiled, and their whole package is packed as before.
    pub fn collect_graph(&mut self, loaded: Vec<PathBuf>) -> Result<Vec<PathBuf>, String> {
        let mut todo = loaded;
        let mut seen = HashSet::new();
        let mut files = Vec::new();
        while let Some(file) = todo.pop() {
            if !seen.insert(file.clone()) {
                continue;
            }
            files.push(file.clone());
            if !is_module_source(&file) {
                continue;
            }

            if !self.modules.contains_key(&file) {
                if crate::pack::package_folder(&file).is_none() {
                    return Err(format!(
                        "{} is outside React Compiler source roots; add --react-compiler-root for its project",
                        file.display()
                    ));
                }
                // Batch only the already-resolved frontier. Never scan a whole
                // package: shipped tests and declarations are not runtime edges.
                let mut requested = vec![file.clone()];
                for next in todo.iter().rev() {
                    if requested.len() >= 50 {
                        break;
                    }
                    if !self.modules.contains_key(next)
                        && crate::pack::package_folder(next).is_some()
                        && is_module_source(next)
                        && !requested.contains(next)
                    {
                        requested.push(next.clone());
                    }
                }
                let manifest = run_helper(
                    &self.helper,
                    &self.cache,
                    serde_json::json!({"roots": [], "files": requested, "scanOnly": true}),
                )?;
                for module in manifest["files"]
                    .as_array()
                    .ok_or("module scan has no files")?
                {
                    let input =
                        PathBuf::from(module["input"].as_str().ok_or("module scan has no input")?);
                    self.modules.insert(input, module.clone());
                }
            }

            let module = self
                .modules
                .get(&file)
                .ok_or_else(|| format!("No module graph record for {}", file.display()))?;
            if module["unresolved"]
                .as_array()
                .is_some_and(|items| !items.is_empty())
            {
                return Err(format!(
                    "{} has a nonliteral import/require that cannot be resolved for a standalone build: {}",
                    file.display(),
                    module["unresolved"]
                ));
            }
            for edge in module["imports"].as_array().into_iter().flatten() {
                let name = edge["name"].as_str().ok_or("module edge has no name")?;
                if let Some(found) = crate::bridge::resolve_for_pack(
                    &file,
                    name,
                    edge["import"].as_bool().unwrap_or(true),
                )? {
                    todo.push(found);
                }
            }
        }
        files.extend(crate::bridge::configuration_files_for_pack());
        files.sort();
        files.dedup();
        Ok(files)
    }
}

fn is_module_source(file: &Path) -> bool {
    file.extension().is_some_and(|ext| {
        ["js", "jsx", "ts", "tsx", "mjs", "cjs", "mts", "cts"]
            .iter()
            .any(|one| ext.eq_ignore_ascii_case(one))
    })
}

fn dependency_path(path: &Path) -> bool {
    path.components()
        .any(|part| part.as_os_str().eq_ignore_ascii_case("node_modules"))
}
