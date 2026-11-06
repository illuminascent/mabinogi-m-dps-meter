//! Encounter consolidation only. These intervals do not change HP attribution
//! or its much shorter candidate window. Actor roles form an undirected graph;
//! identity aliases, owner grouping and shared names never create edges.
use super::{Hit,HitAssignment};
use crate::identity::EntityKey;
use std::collections::{BTreeMap,BTreeSet};

struct Components { parent:Vec<usize>,size:Vec<usize> }
impl Components {
    fn root(&mut self,mut n:usize)->usize {
        while self.parent[n]!=n {self.parent[n]=self.parent[self.parent[n]];n=self.parent[n];}n
    }
    fn join(&mut self,a:usize,b:usize) {
        let (mut a,mut b)=(self.root(a),self.root(b));if a==b{return;}
        if self.size[a]<self.size[b]{std::mem::swap(&mut a,&mut b);}
        self.parent[b]=a;self.size[a]+=self.size[b];
    }
}

pub(super) fn connected(mut hits:Vec<(&Hit,&HitAssignment,bool)>,joining:&Hit,idle_ns:u64)->Vec<(Hit,HitAssignment,bool)> {
    hits.sort_by_key(|(h,_,_)|h.timestamp_ns);
    let mut components=Components{parent:(0..hits.len()).collect(),size:vec![1;hits.len()]};
    let mut nodes:BTreeMap<EntityKey,(u64,usize)>=BTreeMap::new();
    for (i,(h,_,_)) in hits.iter().enumerate() {
        for node in [h.source,h.target] {
            if let Some((last,previous))=nodes.insert(node,(h.timestamp_ns,i)) {
                if h.timestamp_ns.saturating_sub(last)<idle_ns {components.join(i,previous);}
            }
        }
    }
    let roots:BTreeSet<_>=[joining.source,joining.target].into_iter().filter_map(|node|nodes.get(&node))
        .filter(|(last,_)|last.abs_diff(joining.timestamp_ns)<idle_ns).map(|(_,i)|components.root(*i)).collect();
    hits.into_iter().enumerate().filter(|(i,_)|roots.contains(&components.root(*i)))
        .map(|(_, (h,a,inferred))|(h.clone(),a.clone(),inferred)).collect()
}
