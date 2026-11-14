use ignore::gitignore::{Gitignore, GitignoreBuilder};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RuleError {
    pub pattern: String,
    pub message: String,
}

pub struct HideRules {
    matcher: Option<Gitignore>,
}

impl HideRules {
    pub fn new(global: &[String], repository: &[String]) -> (HideRules, Vec<RuleError>) {
        let mut builder = GitignoreBuilder::new("");
        let mut errors = Vec::new();
        let mut any = false;
        for pattern in global.iter().chain(repository) {
            let line = pattern.trim_end();
            if line.trim().is_empty() || line.starts_with('#') {
                continue;
            }
            match builder.add_line(None, line) {
                Ok(_) => any = true,
                Err(err) => errors.push(RuleError {
                    pattern: line.to_string(),
                    message: err.to_string(),
                }),
            }
        }
        let matcher = if any {
            match builder.build() {
                Ok(matcher) => Some(matcher),
                Err(err) => {
                    errors.push(RuleError {
                        pattern: String::new(),
                        message: err.to_string(),
                    });
                    None
                }
            }
        } else {
            None
        };
        (HideRules { matcher }, errors)
    }

    pub fn is_empty(&self) -> bool {
        self.matcher.is_none()
    }

    pub fn matching_rule(&self, path: &str) -> Option<String> {
        let matcher = self.matcher.as_ref()?;
        let path = path.trim_start_matches('/');
        if path.is_empty() || std::path::Path::new(path).has_root() {
            return None;
        }
        match matcher.matched_path_or_any_parents(path, false) {
            ignore::Match::Ignore(glob) => Some(glob.original().to_string()),
            _ => None,
        }
    }

    pub fn is_hidden(&self, path: &str) -> bool {
        self.matching_rule(path).is_some()
    }
}

pub fn escape(literal: &str) -> String {
    let mut out = String::with_capacity(literal.len() + 2);
    for (ix, c) in literal.chars().enumerate() {
        if matches!(c, '\\' | '*' | '?' | '[' | ']' | '{' | '}')
            || (ix == 0 && matches!(c, '!' | '#'))
        {
            out.push('\\');
        }
        out.push(c);
    }
    out
}

pub fn file_pattern(path: &str) -> String {
    format!("/{}", escape(path.trim_matches('/')))
}

pub fn dir_pattern(path: &str) -> String {
    format!("/{}/", escape(path.trim_matches('/')))
}

pub fn dir_name_pattern(path: &str) -> String {
    let name = path
        .trim_matches('/')
        .rsplit('/')
        .next()
        .unwrap_or_default();
    format!("{}/", escape(name))
}

pub fn family_pattern(path: &str) -> Option<String> {
    let name = path.rsplit('/').next().unwrap_or(path);
    let (stem, ext) = name.split_once('.').filter(|(stem, _)| !stem.is_empty())?;
    let suffix = match stem.rfind(['_', '-']) {
        Some(at) if !ext.contains('.') && at > 0 && at + 1 < stem.len() => {
            format!("{}.{ext}", &stem[at..])
        }
        _ => format!(".{ext}"),
    };
    Some(format!("*{}", escape(&suffix)))
}

pub fn merge_patterns(list: &mut Vec<String>, baseline: &[String], typed: &[String]) {
    if list.as_slice() == baseline {
        *list = typed.to_vec();
        return;
    }
    list.retain(|p| typed.contains(p) || !baseline.contains(p));
    for pattern in typed {
        if !list.contains(pattern) {
            list.push(pattern.clone());
        }
    }
}
