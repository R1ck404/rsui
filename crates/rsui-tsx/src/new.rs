//! Starting a new app: `rsui-tsx new my-app`. It makes a folder with what it takes to run an app, check it, and have an agent work on it.

use std::path::{Path, PathBuf};

const APP: &str = include_str!("../template/app.tsx");
const AGENTS: &str = include_str!("../template/AGENTS.md");
const TYPES: &str = include_str!("../rsui.d.ts");
const TSCONFIG: &str = r#"{
  "compilerOptions": {
    "noEmit": true,
    "strict": true,
    "jsx": "react",
    "jsxFactory": "__jsx",
    "jsxFragmentFactory": "__Fragment",
    "target": "es2022",
    "lib": ["es2022"],
    "types": [],
    "skipLibCheck": true
  },
  "include": ["**/*.ts", "**/*.tsx"]
}
"#;

/// Makes a new app in this folder, which can't already have anything in it, so that nothing can be lost. It says what it made.
pub(crate) fn new_project(place: &Path) -> Result<Vec<PathBuf>, String> {
    let shown = place.display();
    if place.exists() {
        let mut inside = std::fs::read_dir(place).map_err(|error| format!("{shown}: {error}"))?;
        if inside.next().is_some() {
            return Err(format!(
                "{shown} already has things in it, and nothing's been changed. Give the name of a new folder."
            ));
        }
    }
    // The folder's own name, even if it's only been called `.`.
    let whole = if place.exists() {
        std::fs::canonicalize(place).unwrap_or_else(|_| place.to_owned())
    } else {
        place.to_owned()
    };
    let name = whole.file_name().map_or_else(
        || "my-app".to_owned(),
        |name| name.to_string_lossy().into_owned(),
    );

    // As text in a quoted string in the code.
    let in_code = name.replace('\\', "\\\\").replace('\'', "\\'");
    // A server's name is for a config file, and tools get called after it, so it's kept simple.
    let server: String = name
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() {
                character.to_ascii_lowercase()
            } else {
                '-'
            }
        })
        .collect();
    let server = server.trim_matches('-');
    let server = if server.is_empty() { "app" } else { server };
    let mcp = format!(
        "{{\n  \"mcpServers\": {{\n    \"{server}\": {{\n      \"command\": \"rsui-tsx\",\n      \"args\": [\"--agent\", \"app.tsx\"]\n    }}\n  }}\n}}\n"
    );

    // Who the app is, for packing it: an id that's only an example until it's changed to one of the app's own.
    let part = crate::config::id_part(&name);
    let config = [
        "import { defineConfig } from 'rsui/config';".to_owned(),
        String::new(),
        "// Who the app is and how it's packed: see `rsui/config` in rsui.d.ts. `rsui-tsx config` prints what this comes to."
            .to_owned(),
        "export default defineConfig({".to_owned(),
        format!("    // Change this to one of your own, as `com.yourname.{part}`: two apps with the same id replace each other."),
        format!("    id: 'com.example.{part}',"),
        format!("    name: '{in_code}',"),
        "    version: '0.1.0',".to_owned(),
        "});".to_owned(),
        String::new(),
    ]
    .join("\n");
    let files: [(&str, String); 8] = [
        ("app.tsx", APP.replace("{{TITLE}}", &in_code)),
        ("rsui.config.ts", config),
        ("rsui.d.ts", TYPES.to_owned()),
        ("tsconfig.json", TSCONFIG.to_owned()),
        (".mcp.json", mcp),
        ("AGENTS.md", AGENTS.replace("{{TITLE}}", &name)),
        // Claude's name for the same thing, which reads this one.
        ("CLAUDE.md", "@AGENTS.md\n".to_owned()),
        (".gitignore", "*.exe\n*.png\n".to_owned()),
    ];
    std::fs::create_dir_all(place).map_err(|error| format!("{shown}: {error}"))?;
    let mut made = Vec::new();
    for (file, text) in files {
        let path = place.join(file);
        std::fs::write(&path, text).map_err(|error| format!("{}: {error}", path.display()))?;
        made.push(path);
    }
    Ok(made)
}
