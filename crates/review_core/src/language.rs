pub fn language_override(path: &str) -> Option<&'static str> {
    let name = path.rsplit(['/', '\\']).next().unwrap_or(path);
    let name = name.to_ascii_lowercase();
    if let Some(language) = by_file_name(&name) {
        return Some(language);
    }
    if name.starts_with("makefile.") || name.starts_with("gnumakefile.") {
        return Some("make");
    }
    let (_, extension) = name.rsplit_once('.')?;
    by_extension(extension)
}

fn by_file_name(name: &str) -> Option<&'static str> {
    let language = match name {
        "bsdmakefile" => "make",
        ".bashrc" | ".bash_profile" | ".bash_login" | ".bash_logout" | ".bash_aliases"
        | ".zshrc" | ".zshenv" | ".zprofile" | ".zlogin" | ".zlogout" | ".profile" | ".envrc"
        | "pkgbuild" | "apkbuild" => "bash",
        "rakefile" | "gemfile" | "podfile" | "fastfile" | "appfile" | "vagrantfile"
        | "brewfile" | "guardfile" | "capfile" | "dangerfile" | "berksfile" | "thorfile" => "ruby",
        "cargo.lock" | "poetry.lock" | "uv.lock" | "pdm.lock" | "pipfile" => "toml",
        "sconstruct" | "sconscript" | "build.bazel" | "workspace" | "workspace.bazel"
        | "module.bazel" | "tiltfile" | "snakefile" => "python",
        ".babelrc" | ".swcrc" | ".jshintrc" | "flake.lock" | "composer.lock" => "json",
        ".clang-format" | ".clang-tidy" | ".clangd" | ".yamllint" => "yaml",
        ".luacheckrc" => "lua",
        "mix.lock" => "elixir",
        _ => return None,
    };
    Some(language)
}

fn by_extension(extension: &str) -> Option<&'static str> {
    let language = match extension {
        "mk" | "mak" | "make" => "make",
        "zsh" | "ksh" | "bats" | "ebuild" | "eclass" => "bash",
        "hh" | "h++" | "c++" | "ipp" | "tpp" | "inl" | "cppm" | "ixx" | "cu" | "cuh" => "cpp",
        "csx" => "csharp",
        "ktm" => "kotlin",
        "sc" | "sbt" => "scala",
        "rake" | "gemspec" | "podspec" | "ru" | "rbi" | "jbuilder" | "thor" => "ruby",
        "pyw" | "bzl" | "bazel" | "star" => "python",
        "jsonl" | "ndjson" | "json5" | "geojson" | "webmanifest" | "code-workspace" => "json",
        "markdown" | "mkd" | "mkdn" | "mdown" => "markdown",
        "xhtml" | "shtml" => "html",
        "patch" => "diff",
        "graphqls" | "gqls" => "graphql",
        "zon" => "zig",
        "rockspec" | "luau" => "lua",
        "pgsql" | "psql" | "mysql" => "sql",
        _ => return None,
    };
    Some(language)
}
