//! CFG analyses: successors, dominators, dominance frontiers, natural loops.

use super::ir::{BasicBlock, BlockId, SsaFunction, Terminator};
use std::collections::{HashMap, HashSet};

pub fn rebuild_edges(func: &mut SsaFunction) {
    let ids: Vec<BlockId> = func.blocks.keys().copied().collect();
    for id in &ids {
        func.block_mut(*id).preds.clear();
        func.block_mut(*id).succs.clear();
    }
    for id in ids {
        let succs = term_succs(&func.block(id).term);
        func.block_mut(id).succs = succs.clone();
        for s in succs {
            if let Some(b) = func.blocks.get_mut(&s) {
                if !b.preds.contains(&id) {
                    b.preds.push(id);
                }
            }
        }
    }
}

pub fn term_succs(term: &Terminator) -> Vec<BlockId> {
    match term {
        Terminator::Br(t) => vec![*t],
        Terminator::CondBr {
            then_bb, else_bb, ..
        } => vec![*then_bb, *else_bb],
        Terminator::Return(_)
        | Terminator::Halt
        | Terminator::Throw(_)
        | Terminator::Unreachable => vec![],
    }
}

/// Lengauer-Tarjan-style iterative dominators (simple dataflow).
pub fn compute_dominators(func: &SsaFunction) -> HashMap<BlockId, HashSet<BlockId>> {
    let all: HashSet<BlockId> = func.blocks.keys().copied().collect();
    let mut dom: HashMap<BlockId, HashSet<BlockId>> = HashMap::new();
    for &b in &all {
        if b == func.entry {
            dom.insert(b, HashSet::from([b]));
        } else {
            dom.insert(b, all.clone());
        }
    }
    let mut changed = true;
    while changed {
        changed = false;
        for &b in &all {
            if b == func.entry {
                continue;
            }
            let preds = &func.block(b).preds;
            if preds.is_empty() {
                continue;
            }
            let mut new_dom = all.clone();
            for p in preds {
                if let Some(pd) = dom.get(p) {
                    new_dom = new_dom.intersection(pd).copied().collect();
                }
            }
            new_dom.insert(b);
            if new_dom != dom[&b] {
                dom.insert(b, new_dom);
                changed = true;
            }
        }
    }
    dom
}

pub fn immediate_dominator(dom: &HashMap<BlockId, HashSet<BlockId>>, b: BlockId) -> Option<BlockId> {
    let dset = dom.get(&b)?;
    // idom = unique strict dominator that is dominated by all other strict dominators
    let strict: Vec<BlockId> = dset.iter().copied().filter(|&d| d != b).collect();
    for &cand in &strict {
        if strict.iter().all(|&d| d == cand || dom.get(&d).map(|s| s.contains(&cand)).unwrap_or(false)) {
            // cand is dominated by all other strict dominators? We want the closest.
            // Better: cand such that every other strict dominator dominates cand.
            if strict
                .iter()
                .filter(|&&d| d != cand)
                .all(|&d| dom.get(&cand).map(|s| s.contains(&d)).unwrap_or(false) == false
                    && dom.get(&d).map(|s| s.contains(&cand)).unwrap_or(false))
            {
                return Some(cand);
            }
        }
    }
    // Fallback: pick any with largest dom set among strict
    strict
        .into_iter()
        .max_by_key(|d| dom.get(d).map(|s| s.len()).unwrap_or(0))
}

pub fn compute_idom(func: &SsaFunction) -> HashMap<BlockId, BlockId> {
    let dom = compute_dominators(func);
    let mut idom = HashMap::new();
    for &b in func.blocks.keys() {
        if b == func.entry {
            continue;
        }
        if let Some(i) = find_idom(&dom, b) {
            idom.insert(b, i);
        }
    }
    idom
}

fn find_idom(dom: &HashMap<BlockId, HashSet<BlockId>>, b: BlockId) -> Option<BlockId> {
    let dset = dom.get(&b)?;
    let strict: Vec<BlockId> = dset.iter().copied().filter(|&d| d != b).collect();
    // idom(b) is the unique member of strict that is dominated by every other member
    for &cand in &strict {
        if strict
            .iter()
            .filter(|&&d| d != cand)
            .all(|&d| dom.get(&cand).map(|s| s.contains(&d)).unwrap_or(false))
        {
            return Some(cand);
        }
    }
    strict.first().copied()
}

pub fn dominance_frontiers(func: &SsaFunction) -> HashMap<BlockId, HashSet<BlockId>> {
    let idom = compute_idom(func);
    let mut df: HashMap<BlockId, HashSet<BlockId>> =
        func.blocks.keys().map(|&b| (b, HashSet::new())).collect();
    for &b in func.blocks.keys() {
        let preds = &func.block(b).preds;
        if preds.len() < 2 {
            continue;
        }
        for &p in preds {
            let mut runner = p;
            while Some(&runner) != idom.get(&b) {
                df.entry(runner).or_default().insert(b);
                if let Some(&i) = idom.get(&runner) {
                    if i == runner {
                        break;
                    }
                    runner = i;
                } else {
                    break;
                }
            }
        }
    }
    df
}

#[derive(Debug, Clone)]
pub struct LoopInfo {
    pub header: BlockId,
    pub body: HashSet<BlockId>,
    pub latches: Vec<BlockId>,
    pub exits: Vec<BlockId>,
}

/// Natural loops from back-edges (header dominates latch).
pub fn find_loops(func: &SsaFunction) -> Vec<LoopInfo> {
    let dom = compute_dominators(func);
    let mut loops = Vec::new();
    for (&latch, block) in &func.blocks {
        for &succ in &block.succs {
            if dom.get(&latch).map(|d| d.contains(&succ)).unwrap_or(false) {
                // back-edge latch -> succ (header)
                let header = succ;
                let mut body = HashSet::from([header]);
                let mut stack = vec![latch];
                body.insert(latch);
                while let Some(n) = stack.pop() {
                    for &p in &func.block(n).preds {
                        if body.insert(p) {
                            stack.push(p);
                        }
                    }
                }
                let latches = vec![latch];
                let mut exits = Vec::new();
                for &b in &body {
                    for &s in &func.block(b).succs {
                        if !body.contains(&s) {
                            exits.push(s);
                        }
                    }
                }
                loops.push(LoopInfo {
                    header,
                    body,
                    latches,
                    exits,
                });
            }
        }
    }
    loops
}

pub fn block_rpo(func: &SsaFunction) -> Vec<BlockId> {
    let mut visited = HashSet::new();
    let mut post = Vec::new();
    fn dfs(
        func: &SsaFunction,
        b: BlockId,
        visited: &mut HashSet<BlockId>,
        post: &mut Vec<BlockId>,
    ) {
        if !visited.insert(b) {
            return;
        }
        for s in func.block(b).succs.clone() {
            dfs(func, s, visited, post);
        }
        post.push(b);
    }
    dfs(func, func.entry, &mut visited, &mut post);
    post.reverse();
    post
}

pub fn reachable(func: &SsaFunction) -> HashSet<BlockId> {
    let mut vis = HashSet::new();
    let mut stack = vec![func.entry];
    while let Some(b) = stack.pop() {
        if !vis.insert(b) {
            continue;
        }
        for s in &func.block(b).succs {
            stack.push(*s);
        }
    }
    vis
}

pub fn dominates(dom: &HashMap<BlockId, HashSet<BlockId>>, a: BlockId, b: BlockId) -> bool {
    dom.get(&b).map(|s| s.contains(&a)).unwrap_or(false)
}

#[allow(dead_code)]
pub fn block_inst_count(b: &BasicBlock) -> usize {
    b.insts.len()
}
