//! Starter files offered when initializing a repository.

/// (name, .gitignore contents)
pub const GITIGNORE: &[(&str, &str)] = &[
    (
        "Rust",
        "/target/\n**/*.rs.bk\n*.pdb\n",
    ),
    (
        "Node",
        "node_modules/\nnpm-debug.log*\nyarn-debug.log*\nyarn-error.log*\npnpm-debug.log*\n\
         dist/\nbuild/\ncoverage/\n.env\n.env.local\n.cache/\n",
    ),
    (
        "Python",
        "__pycache__/\n*.py[cod]\n*.egg-info/\n.eggs/\nbuild/\ndist/\n.venv/\nvenv/\nenv/\n\
         .pytest_cache/\n.mypy_cache/\n.ruff_cache/\n.coverage\nhtmlcov/\n.ipynb_checkpoints/\n.env\n",
    ),
    (
        "Go",
        "*.exe\n*.exe~\n*.dll\n*.so\n*.dylib\n*.test\n*.out\ngo.work\ngo.work.sum\nvendor/\n",
    ),
    (
        "Java",
        "*.class\n*.jar\n*.war\n*.ear\n*.log\ntarget/\nbuild/\n.gradle/\nout/\n\
         hs_err_pid*\n",
    ),
    (
        "C# / .NET",
        "bin/\nobj/\n*.user\n*.suo\n*.userosscache\n*.sln.docstates\n.vs/\n\
         [Dd]ebug/\n[Rr]elease/\nTestResults/\n*.nupkg\npackages/\n",
    ),
    (
        "C / C++",
        "*.o\n*.obj\n*.a\n*.lib\n*.so\n*.dylib\n*.dll\n*.exe\n*.out\nbuild/\ncmake-build-*/\n\
         CMakeCache.txt\nCMakeFiles/\ncompile_commands.json\n",
    ),
    (
        "Swift / Xcode",
        "xcuserdata/\n*.xcuserstate\nDerivedData/\n.build/\n.swiftpm/\nPods/\nCarthage/Build/\n",
    ),
    (
        "Unity",
        "/[Ll]ibrary/\n/[Tt]emp/\n/[Oo]bj/\n/[Bb]uild/\n/[Bb]uilds/\n/[Ll]ogs/\n/[Uu]ser[Ss]ettings/\n\
         *.csproj\n*.sln\n*.unityproj\n*.pidb\n*.booproj\n",
    ),
];

/// Operating-system and editor files, appended to any template.
pub const GITIGNORE_COMMON: &str =
    "\n# Operating system and editor files\n.DS_Store\nThumbs.db\nDesktop.ini\n.idea/\n.vscode/\n*.swp\n*~\n";

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum License {
    Mit,
    Bsd3,
}

impl License {
    pub const ALL: [Self; 2] = [Self::Mit, Self::Bsd3];

    pub fn label(self) -> &'static str {
        match self {
            Self::Mit => "MIT",
            Self::Bsd3 => "BSD 3-Clause",
        }
    }

    pub fn text(self, year: i32, holder: &str) -> String {
        match self {
            Self::Mit => format!(
                "MIT License\n\nCopyright (c) {year} {holder}\n\n\
Permission is hereby granted, free of charge, to any person obtaining a copy\n\
of this software and associated documentation files (the \"Software\"), to deal\n\
in the Software without restriction, including without limitation the rights\n\
to use, copy, modify, merge, publish, distribute, sublicense, and/or sell\n\
copies of the Software, and to permit persons to whom the Software is\n\
furnished to do so, subject to the following conditions:\n\n\
The above copyright notice and this permission notice shall be included in all\n\
copies or substantial portions of the Software.\n\n\
THE SOFTWARE IS PROVIDED \"AS IS\", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR\n\
IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,\n\
FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE\n\
AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER\n\
LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,\n\
OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE\n\
SOFTWARE.\n"
            ),
            Self::Bsd3 => format!(
                "BSD 3-Clause License\n\nCopyright (c) {year}, {holder}\n\n\
Redistribution and use in source and binary forms, with or without\n\
modification, are permitted provided that the following conditions are met:\n\n\
1. Redistributions of source code must retain the above copyright notice, this\n   \
list of conditions and the following disclaimer.\n\n\
2. Redistributions in binary form must reproduce the above copyright notice,\n   \
this list of conditions and the following disclaimer in the documentation\n   \
and/or other materials provided with the distribution.\n\n\
3. Neither the name of the copyright holder nor the names of its\n   \
contributors may be used to endorse or promote products derived from\n   \
this software without specific prior written permission.\n\n\
THIS SOFTWARE IS PROVIDED BY THE COPYRIGHT HOLDERS AND CONTRIBUTORS \"AS IS\"\n\
AND ANY EXPRESS OR IMPLIED WARRANTIES, INCLUDING, BUT NOT LIMITED TO, THE\n\
IMPLIED WARRANTIES OF MERCHANTABILITY AND FITNESS FOR A PARTICULAR PURPOSE ARE\n\
DISCLAIMED. IN NO EVENT SHALL THE COPYRIGHT HOLDER OR CONTRIBUTORS BE LIABLE\n\
FOR ANY DIRECT, INDIRECT, INCIDENTAL, SPECIAL, EXEMPLARY, OR CONSEQUENTIAL\n\
DAMAGES (INCLUDING, BUT NOT LIMITED TO, PROCUREMENT OF SUBSTITUTE GOODS OR\n\
SERVICES; LOSS OF USE, DATA, OR PROFITS; OR BUSINESS INTERRUPTION) HOWEVER\n\
CAUSED AND ON ANY THEORY OF LIABILITY, WHETHER IN CONTRACT, STRICT LIABILITY,\n\
OR TORT (INCLUDING NEGLIGENCE OR OTHERWISE) ARISING IN ANY WAY OUT OF THE USE\n\
OF THIS SOFTWARE, EVEN IF ADVISED OF THE POSSIBILITY OF SUCH DAMAGE.\n"
            ),
        }
    }
}

/// Choices from the Initialize Repository dialog.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct InitOptions {
    pub default_branch: String,
    /// Index into [`GITIGNORE`].
    pub gitignore: Option<usize>,
    pub readme: bool,
    pub license: Option<License>,
    /// Copyright holder for the license.
    pub author: String,
    pub initial_commit: bool,
}

impl Default for InitOptions {
    fn default() -> Self {
        Self {
            default_branch: "main".into(),
            gitignore: None,
            readme: false,
            license: None,
            author: String::new(),
            initial_commit: false,
        }
    }
}

fn current_year() -> i32 {
    let days = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() / 86_400)
        .unwrap_or(0) as i64;
    // Civil-from-days (Howard Hinnant), proleptic Gregorian calendar.
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let month = if mp < 10 { mp + 3 } else { mp - 9 };
    (yoe + era * 400 + i64::from(month <= 2)) as i32
}

/// Files to create, as (relative path, contents).
pub fn files(options: &InitOptions, repository_name: &str) -> Vec<(&'static str, String)> {
    let mut files = Vec::new();
    if let Some((_, body)) = options.gitignore.and_then(|i| GITIGNORE.get(i)) {
        files.push((".gitignore", format!("{body}{GITIGNORE_COMMON}")));
    }
    if options.readme {
        files.push(("README.md", format!("# {repository_name}\n")));
    }
    if let Some(license) = options.license {
        let holder = options.author.trim();
        let holder = if holder.is_empty() {
            "the authors"
        } else {
            holder
        };
        files.push(("LICENSE", license.text(current_year(), holder)));
    }
    files
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn current_year_is_plausible() {
        assert!((2024..2200).contains(&current_year()));
    }

    #[test]
    fn files_follow_the_selected_options() {
        assert!(files(&InitOptions::default(), "demo").is_empty());
        let options = InitOptions {
            gitignore: Some(0),
            readme: true,
            license: Some(License::Mit),
            author: "Ada Lovelace".into(),
            ..Default::default()
        };
        let files = files(&options, "demo");
        let names: Vec<_> = files.iter().map(|(name, _)| *name).collect();
        assert_eq!(names, [".gitignore", "README.md", "LICENSE"]);
        assert!(files[0].1.starts_with("/target/") && files[0].1.contains(".DS_Store"));
        assert_eq!(files[1].1, "# demo\n");
        assert!(files[2].1.contains("Ada Lovelace") && files[2].1.starts_with("MIT License"));
    }
}
