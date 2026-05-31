use std::path::Path;

#[derive(Debug, Clone)]
pub struct SubmoduleInfo {
    pub name: String,
    pub path: String,
    pub url: String,
    pub initialized: bool,
}

/// Parse `.gitmodules` and return one `SubmoduleInfo` per submodule.
/// Returns an empty Vec if `.gitmodules` is absent or unparseable.
pub fn list(repo_path: &Path) -> Vec<SubmoduleInfo> {
    let modules_path = repo_path.join(".gitmodules");
    let content = match std::fs::read_to_string(&modules_path) {
        Ok(c) => c,
        Err(_) => return Vec::new(),
    };

    let mut result = Vec::new();
    let mut current_name: Option<String> = None;
    let mut current_path: Option<String> = None;
    let mut current_url: Option<String> = None;

    for line in content.lines() {
        let trimmed = line.trim();

        // Section header: [submodule "name"]
        if trimmed.starts_with("[submodule") {
            // Flush previous entry
            if let (Some(name), Some(path), Some(url)) =
                (current_name.take(), current_path.take(), current_url.take())
            {
                let initialized = repo_path.join(&path).join(".git").exists();
                result.push(SubmoduleInfo { name, path, url, initialized });
            }

            // Extract name between quotes
            let name = trimmed
                .trim_start_matches("[submodule")
                .trim()
                .trim_matches('"')
                .trim_end_matches(']')
                .trim_matches('"')
                .to_string();
            current_name = Some(name);
            current_path = None;
            current_url = None;
            continue;
        }

        // Key = value entries
        if let Some(eq) = trimmed.find('=') {
            let key = trimmed[..eq].trim();
            let val = trimmed[eq + 1..].trim().to_string();
            match key {
                "path" => current_path = Some(val),
                "url"  => current_url  = Some(val),
                _ => {}
            }
        }
    }

    // Flush the last entry
    if let (Some(name), Some(path), Some(url)) =
        (current_name.take(), current_path.take(), current_url.take())
    {
        let initialized = repo_path.join(&path).join(".git").exists();
        result.push(SubmoduleInfo { name, path, url, initialized });
    }

    result
}
