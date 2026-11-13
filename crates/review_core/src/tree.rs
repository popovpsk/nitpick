use std::collections::{BTreeMap, HashSet};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileStatus {
    Added,
    Deleted,
    Modified,
    Renamed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreeFile {
    pub path: String,
    pub status: FileStatus,
    pub additions: u32,
    pub deletions: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum NodeKind {
    Dir { children: Vec<usize> },
    File { file: usize },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Node {
    pub name: String,
    pub path: String,
    pub parent: Option<usize>,
    pub depth: usize,
    pub kind: NodeKind,
}

impl Node {
    pub fn is_dir(&self) -> bool {
        matches!(self.kind, NodeKind::Dir { .. })
    }

    pub fn file(&self) -> Option<usize> {
        match self.kind {
            NodeKind::File { file } => Some(file),
            NodeKind::Dir { .. } => None,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Row {
    pub node: usize,
    pub depth: usize,
    pub expanded: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Progress {
    pub viewed: usize,
    pub total: usize,
}

impl Progress {
    pub fn is_complete(self) -> bool {
        self.total > 0 && self.viewed == self.total
    }

    pub fn is_partial(self) -> bool {
        self.viewed > 0 && self.viewed < self.total
    }
}

#[derive(Debug, Clone, Default)]
pub struct FileTree {
    nodes: Vec<Node>,
    roots: Vec<usize>,
    file_nodes: Vec<usize>,
    order: Vec<usize>,
}

#[derive(Default)]
struct Builder {
    dirs: BTreeMap<String, Builder>,
    files: Vec<(String, usize)>,
}

impl FileTree {
    pub fn build(files: &[TreeFile]) -> Self {
        let mut root = Builder::default();
        for (ix, file) in files.iter().enumerate() {
            let mut parts: Vec<&str> = file.path.split('/').filter(|p| !p.is_empty()).collect();
            let Some(name) = parts.pop() else {
                continue;
            };
            let mut dir = &mut root;
            for part in parts {
                dir = dir.dirs.entry(part.to_string()).or_default();
            }
            dir.files.push((name.to_string(), ix));
        }
        let mut tree = FileTree {
            file_nodes: vec![usize::MAX; files.len()],
            ..FileTree::default()
        };
        let roots = tree.add_children(root, None, "", 0);
        tree.roots = roots;
        let mut order = Vec::with_capacity(files.len());
        for &root in &tree.roots {
            tree.collect_files(root, &mut order);
        }
        tree.order = order;
        tree
    }

    fn add_children(
        &mut self,
        builder: Builder,
        parent: Option<usize>,
        prefix: &str,
        depth: usize,
    ) -> Vec<usize> {
        let mut ids = Vec::with_capacity(builder.dirs.len() + builder.files.len());
        let mut dirs: Vec<(String, Builder)> = builder.dirs.into_iter().collect();
        dirs.sort_by(|a, b| natural_key(&a.0).cmp(&natural_key(&b.0)));
        for (name, mut sub) in dirs {
            let mut display = name;
            while sub.files.is_empty() && sub.dirs.len() == 1 {
                let (child_name, child) = sub.dirs.pop_first().unwrap_or_default();
                display = format!("{display}/{child_name}");
                sub = child;
            }
            let path = join(prefix, &display);
            let id = self.nodes.len();
            self.nodes.push(Node {
                name: display,
                path: path.clone(),
                parent,
                depth,
                kind: NodeKind::Dir {
                    children: Vec::new(),
                },
            });
            let children = self.add_children(sub, Some(id), &path, depth + 1);
            self.nodes[id].kind = NodeKind::Dir { children };
            ids.push(id);
        }
        let mut files = builder.files;
        files.sort_by(|a, b| natural_key(&a.0).cmp(&natural_key(&b.0)));
        for (name, file) in files {
            let id = self.nodes.len();
            self.nodes.push(Node {
                path: join(prefix, &name),
                name,
                parent,
                depth,
                kind: NodeKind::File { file },
            });
            self.file_nodes[file] = id;
            ids.push(id);
        }
        ids
    }

    fn collect_files(&self, node: usize, out: &mut Vec<usize>) {
        match &self.nodes[node].kind {
            NodeKind::File { file } => out.push(*file),
            NodeKind::Dir { children } => {
                for &child in children {
                    self.collect_files(child, out);
                }
            }
        }
    }

    pub fn nodes(&self) -> &[Node] {
        &self.nodes
    }

    pub fn node(&self, ix: usize) -> &Node {
        &self.nodes[ix]
    }

    pub fn roots(&self) -> &[usize] {
        &self.roots
    }

    pub fn is_empty(&self) -> bool {
        self.nodes.is_empty()
    }

    pub fn file_node(&self, file: usize) -> Option<usize> {
        self.file_nodes
            .get(file)
            .copied()
            .filter(|&n| n != usize::MAX)
    }

    pub fn file_order(&self) -> &[usize] {
        &self.order
    }

    pub fn dir_paths(&self) -> impl Iterator<Item = &str> {
        self.nodes
            .iter()
            .filter(|n| n.is_dir())
            .map(|n| n.path.as_str())
    }

    pub fn rows(&self, collapsed: &HashSet<String>) -> Vec<Row> {
        let mut rows = Vec::with_capacity(self.nodes.len());
        let mut stack: Vec<usize> = self.roots.iter().rev().copied().collect();
        while let Some(ix) = stack.pop() {
            let node = &self.nodes[ix];
            let expanded = match &node.kind {
                NodeKind::Dir { children } => {
                    let expanded = !collapsed.contains(&node.path);
                    if expanded {
                        stack.extend(children.iter().rev().copied());
                    }
                    expanded
                }
                NodeKind::File { .. } => false,
            };
            rows.push(Row {
                node: ix,
                depth: node.depth,
                expanded,
            });
        }
        rows
    }

    pub fn ancestors(&self, node: usize) -> impl Iterator<Item = &Node> {
        std::iter::successors(self.nodes[node].parent, |&p| self.nodes[p].parent)
            .map(|ix| &self.nodes[ix])
    }

    pub fn progress(&self, viewed: &[bool]) -> Vec<Progress> {
        let mut progress = vec![Progress::default(); self.nodes.len()];
        for ix in (0..self.nodes.len()).rev() {
            progress[ix] = match &self.nodes[ix].kind {
                NodeKind::File { file } => Progress {
                    viewed: usize::from(viewed.get(*file).copied().unwrap_or(false)),
                    total: 1,
                },
                NodeKind::Dir { children } => {
                    children
                        .iter()
                        .fold(Progress::default(), |acc, &child| Progress {
                            viewed: acc.viewed + progress[child].viewed,
                            total: acc.total + progress[child].total,
                        })
                }
            };
        }
        progress
    }

    pub fn files_under(&self, node: usize) -> Vec<usize> {
        let mut out = Vec::new();
        self.collect_files(node, &mut out);
        out
    }
}

fn join(prefix: &str, name: &str) -> String {
    if prefix.is_empty() {
        name.to_string()
    } else {
        format!("{prefix}/{name}")
    }
}

fn natural_key(name: &str) -> (String, &str) {
    (name.to_lowercase(), name)
}
