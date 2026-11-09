//! Nests stacked PRs: a PR whose base branch is another open PR's head branch is shown
//! under that PR, with file-tree style connectors.

use crate::gh::{Item, Kind};
use std::collections::HashMap;

/// Where one input item goes in the display order, and the connector text before it.
#[derive(Debug, PartialEq, Eq)]
pub struct Placed {
    pub index: usize,
    pub prefix: String,
}

/// Nesting beyond this depth keeps the same indentation so rows never run out of width.
const MAX_DEPTH: usize = 4;

/// Arranges `items` into display order. The input must already be grouped by repo and sorted
/// as it should appear among siblings; roots and children keep that relative order.
pub fn arrange(items: &[&Item]) -> Vec<Placed> {
    let parents = find_parents(items);
    let mut children: Vec<Vec<usize>> = vec![vec![]; items.len()];
    for (child, parent) in parents.iter().enumerate() {
        if let Some(p) = parent {
            children[*p].push(child);
        }
    }
    let mut out = Vec::with_capacity(items.len());
    for root in (0..items.len()).filter(|i| parents[*i].is_none()) {
        out.push(Placed {
            index: root,
            prefix: String::new(),
        });
        emit_children(root, &children, "", 1, &mut out);
    }
    out
}

fn emit_children(
    node: usize,
    children: &[Vec<usize>],
    trail: &str,
    depth: usize,
    out: &mut Vec<Placed>,
) {
    let kids = &children[node];
    for (n, kid) in kids.iter().enumerate() {
        let last = n + 1 == kids.len();
        let connector = if last { "└─" } else { "├─" };
        out.push(Placed {
            index: *kid,
            prefix: format!("{trail}{connector}"),
        });
        let next_trail = if depth >= MAX_DEPTH {
            trail.to_string()
        } else if last {
            format!("{trail}  ")
        } else {
            format!("{trail}│ ")
        };
        emit_children(*kid, children, &next_trail, depth + 1, out);
    }
}

/// For each item, the index of the PR it is stacked on, if that PR is in `items`.
fn find_parents(items: &[&Item]) -> Vec<Option<usize>> {
    // First match wins, so with duplicate head names the earlier (more recently updated) PR is used.
    let mut by_head: HashMap<(&str, &str), usize> = HashMap::new();
    for (i, item) in items.iter().enumerate() {
        if item.kind == Kind::Pr && !item.head.is_empty() {
            by_head
                .entry((item.repo_full.as_str(), item.head.as_str()))
                .or_insert(i);
        }
    }
    let mut parents: Vec<Option<usize>> = items
        .iter()
        .enumerate()
        .map(|(i, item)| {
            if item.kind != Kind::Pr || item.base.is_empty() {
                return None;
            }
            by_head
                .get(&(item.repo_full.as_str(), item.base.as_str()))
                .copied()
                .filter(|p| *p != i)
        })
        .collect();
    break_cycles(&mut parents);
    parents
}

/// Makes any PR that sits on a cycle of parents a root, so every PR is reachable from a root.
fn break_cycles(parents: &mut [Option<usize>]) {
    for start in 0..parents.len() {
        let (mut cur, mut steps) = (start, 0);
        while let Some(p) = parents[cur] {
            steps += 1;
            if p == start || steps > parents.len() {
                parents[start] = None;
                break;
            }
            cur = p;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::gh::Ci;
    use chrono::Utc;

    fn pr(repo: &str, number: u64, head: &str, base: &str) -> Item {
        Item {
            id: format!("{repo}{number}"),
            kind: Kind::Pr,
            number,
            title: format!("t{number}"),
            url: String::new(),
            repo: repo.into(),
            repo_full: format!("o/{repo}"),
            author: "a".into(),
            updated_at: Utc::now(),
            draft: false,
            review: None,
            ci: Ci::None,
            gone: None,
            labels: vec![],
            head: head.into(),
            base: base.into(),
            rabbit: None,
        }
    }

    fn layout(items: &[Item]) -> Vec<(u64, String)> {
        let refs: Vec<&Item> = items.iter().collect();
        arrange(&refs)
            .into_iter()
            .map(|p| (items[p.index].number, p.prefix))
            .collect()
    }

    fn row(n: u64, prefix: &str) -> (u64, String) {
        (n, prefix.to_string())
    }

    #[test]
    fn unstacked_prs_stay_flat_in_their_order() {
        let items = [pr("r", 1, "a", "main"), pr("r", 2, "b", "main")];
        assert_eq!(layout(&items), [row(1, ""), row(2, "")]);
    }

    #[test]
    fn a_stack_nests_under_its_base_pr() {
        let items = [
            pr("r", 1, "base-work", "main"),
            pr("r", 2, "mid-work", "base-work"),
            pr("r", 3, "top-work", "mid-work"),
        ];
        assert_eq!(layout(&items), [row(1, ""), row(2, "└─"), row(3, "  └─")]);
    }

    #[test]
    fn siblings_get_branching_connectors_and_trails() {
        let items = [
            pr("r", 1, "root", "main"),
            pr("r", 2, "left", "root"),
            pr("r", 3, "left-child", "left"),
            pr("r", 4, "right", "root"),
        ];
        assert_eq!(
            layout(&items),
            [row(1, ""), row(2, "├─"), row(3, "│ └─"), row(4, "└─")]
        );
    }

    #[test]
    fn children_follow_their_parent_even_when_listed_earlier() {
        let items = [
            pr("r", 9, "child", "parent"),
            pr("r", 5, "parent", "main"),
            pr("r", 7, "other", "main"),
        ];
        assert_eq!(layout(&items), [row(5, ""), row(9, "└─"), row(7, "")]);
    }

    #[test]
    fn bases_that_are_not_open_prs_stay_top_level() {
        let items = [pr("r", 1, "a", "release/1.2"), pr("r", 2, "b", "main")];
        assert_eq!(layout(&items), [row(1, ""), row(2, "")]);
    }

    #[test]
    fn repos_are_independent() {
        let items = [pr("one", 1, "shared", "main"), pr("two", 2, "x", "shared")];
        assert_eq!(layout(&items), [row(1, ""), row(2, "")]);
    }

    #[test]
    fn issues_never_nest() {
        let mut issue = pr("r", 2, "", "");
        issue.kind = Kind::Issue;
        let items = [pr("r", 1, "a", "main"), issue];
        assert_eq!(layout(&items), [row(1, ""), row(2, "")]);
    }

    #[test]
    fn cycles_and_self_stacks_do_not_hang_or_drop_items() {
        let items = [
            pr("r", 1, "a", "b"),
            pr("r", 2, "b", "a"),
            pr("r", 3, "c", "c"),
        ];
        let out = layout(&items);
        assert_eq!(out.len(), 3);
        let mut nums: Vec<u64> = out.iter().map(|r| r.0).collect();
        nums.sort();
        assert_eq!(nums, [1, 2, 3]);
        assert!(out.iter().any(|r| r.1.is_empty()), "a cycle needs a root");
    }

    #[test]
    fn duplicate_head_names_use_the_first_pr() {
        let items = [
            pr("r", 1, "dup", "main"),
            pr("r", 2, "dup", "main"),
            pr("r", 3, "child", "dup"),
        ];
        assert_eq!(layout(&items), [row(1, ""), row(3, "└─"), row(2, "")]);
    }

    #[test]
    fn very_deep_stacks_stop_indenting() {
        let items: Vec<Item> = (0..8)
            .map(|n| {
                pr(
                    "r",
                    n,
                    &format!("b{n}"),
                    &format!("b{}", n.saturating_sub(1)),
                )
            })
            .collect();
        let items: Vec<Item> = std::iter::once(pr("r", 0, "b0", "main"))
            .chain(items.into_iter().skip(1))
            .collect();
        let out = layout(&items);
        let widths: Vec<usize> = out.iter().map(|r| r.1.chars().count()).collect();
        assert!(widths.iter().all(|w| *w <= 2 * MAX_DEPTH), "{widths:?}");
        assert_eq!(out.len(), 8);
    }
}
