//! An effect's node tree with every action typed ([`super::actions::Typed`]): the form a lowering walks.

use super::actions::Typed;
use super::enums::{ConfigType, NodeType};
use super::{Config, Fxr, Node, Result};
use crate::Game;

/// A config of a node with its actions typed.
#[derive(Clone, Debug, PartialEq)]
pub struct TypedConfig {
    /// The config's id (1004 for the usual one).
    pub id: i16,
    /// The config kind the id names, if the fxr project names one.
    pub kind: Option<ConfigType>,
    /// The actions that do something; the empty slots (action type 0) are only counted.
    pub actions: Vec<Typed>,
    /// How many actions of type 0 the config holds.
    pub empty_slots: usize,
}

/// A node with its own actions and its configs' actions typed.
#[derive(Clone, Debug, PartialEq)]
pub struct TypedNode {
    /// The node's id (2000 for the root, 2200 for an ordinary node, ...).
    pub id: i16,
    /// The node kind the id names, if the fxr project names one.
    pub kind: Option<NodeType>,
    /// The node's own actions (the state-config map among them) other than the empty slots.
    pub actions: Vec<Typed>,
    /// How many actions of type 0 the node holds.
    pub empty_slots: usize,
    /// The configs the node's states choose among.
    pub configs: Vec<TypedConfig>,
    /// The child nodes.
    pub children: Vec<TypedNode>,
}

fn actions(list: &[super::Action], game: Game) -> Result<(Vec<Typed>, usize)> {
    let mut typed = Vec::with_capacity(list.len());
    let mut empty = 0usize;
    for a in list {
        if a.id == 0 {
            empty = empty.saturating_add(1);
        } else {
            typed.push(Typed::read(a, game)?);
        }
    }
    Ok((typed, empty))
}

fn config(c: &Config, game: Game) -> Result<TypedConfig> {
    let (actions, empty_slots) = actions(&c.actions, game)?;
    Ok(TypedConfig {
        id: c.id,
        kind: ConfigType::from_value(i32::from(c.id)),
        actions,
        empty_slots,
    })
}

fn node(n: &Node, game: Game) -> Result<TypedNode> {
    let (own, empty_slots) = actions(&n.actions, game)?;
    Ok(TypedNode {
        id: n.id,
        kind: NodeType::from_value(i32::from(n.id)),
        actions: own,
        empty_slots,
        configs: n.configs.iter().map(|c| config(c, game)).collect::<Result<_>>()?,
        children: n.children.iter().map(|c| node(c, game)).collect::<Result<_>>()?,
    })
}

impl TypedNode {
    /// The config index each state chooses (the first field list of the node's state-config map), if it has one.
    pub fn state_configs(&self) -> Option<&[u32]> {
        self.actions.iter().find_map(|a| match a {
            Typed::StateConfigMap(m) => m.extra.field_lists.first().map(Vec::as_slice),
            _ => None,
        })
    }

    /// The config the node runs in `state`: the one its state-config map names (none for an index it lacks), else its only
    /// config.
    pub fn config_in_state(&self, state: usize) -> Option<&TypedConfig> {
        match self.state_configs() {
            Some(map) => self.configs.get(usize::try_from(*map.get(state)?).ok()?),
            None if self.configs.len() == 1 => self.configs.first(),
            None => None,
        }
    }

    /// This node and every node below it, depth first.
    pub fn walk(&self) -> Vec<&TypedNode> {
        let mut out = vec![self];
        for c in &self.children {
            out.extend(c.walk());
        }
        out
    }
}

impl Fxr {
    /// The node tree with every action typed as `game` lays it out. An action no typed struct reads is an error.
    pub fn typed(&self, game: Game) -> Result<TypedNode> {
        node(&self.root, game)
    }
}
