// SPDX-License-Identifier: GPL-3.0-or-later AND Apache-2.0
// Portions ported from IW4L (https://github.com/vladtrc/iw4L) at fc3d8ea,
// crates/asset_anim/src/clip_scheduler.rs, Copyright 2026 vladtrc, licensed under the Apache License 2.0.
// Modifications for DOOMMW2: no global atomic scheduler id (single-threaded); never panics.

//! Weighted clip playback with notetrack crossings (port of iw4L `ClipScheduler`, sheet 10 §5.4).
//!
//! One node per weapon-anim slot. Each node holds a clip, a time, a playback rate, a weight and
//! a goal weight with the time left to reach it. [`ClipScheduler::advance`] steps times, ramps
//! weights, wraps looping clips and reports every notetrack crossed, in time order.

use std::cmp::Ordering;
use std::collections::BinaryHeap;
use std::sync::Arc;

use xmodel_runtime::AnimClip;

/// At most this many notifications are reported per advance (clip_scheduler.rs:7).
const NOTIFY_CAPACITY: usize = 4096;

/// A node index outside the scheduler.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum ClipSchedulerError {
    /// `node` is not below `len`.
    #[error("clip scheduler node {node} is out of range (has {len} nodes)")]
    BadNode {
        /// Requested node.
        node: usize,
        /// Node count.
        len: usize,
    },
}

/// A node that currently contributes to the pose (weight > 0).
#[derive(Debug, Clone, Copy)]
pub struct ActiveAnim<'a> {
    /// Node (weapon-anim slot) index.
    pub node: usize,
    /// The clip playing in the node.
    pub clip: &'a AnimClip,
    /// Clip time, seconds.
    pub time: f32,
    /// Playback rate.
    pub rate: f32,
    /// Blend weight.
    pub weight: f32,
}

/// One notetrack crossed during an advance.
///
/// iw4L also carries a process-wide scheduler id; this single-threaded port has one scheduler
/// per animator and drops it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ClipNotification {
    /// Increments every time a clip is (re)started in any node.
    pub playback: u64,
    /// Node index.
    pub node: usize,
    /// Loop cycle the crossing belongs to.
    pub cycle: i64,
    /// Index into the clip's notifies.
    pub marker: usize,
    /// Notetrack name.
    pub name: String,
}

/// Result of [`ClipScheduler::advance`].
#[derive(Debug, Default)]
pub struct ClipAdvance {
    /// Crossed notetracks in time order.
    pub notifications: Vec<ClipNotification>,
    /// Crossings beyond the reporting capacity.
    pub discarded: u64,
}

#[derive(Debug)]
struct Crossing {
    offset: f64,
    spacing: f64,
    node: usize,
    marker: usize,
    cycle: i64,
    step: i64,
    remaining: u64,
}

impl PartialEq for Crossing {
    fn eq(&self, other: &Self) -> bool {
        self.cmp(other) == Ordering::Equal
    }
}
impl Eq for Crossing {}
impl PartialOrd for Crossing {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}
impl Ord for Crossing {
    // Reversed so the max-heap pops the earliest crossing first.
    fn cmp(&self, other: &Self) -> Ordering {
        other
            .offset
            .total_cmp(&self.offset)
            .then_with(|| other.node.cmp(&self.node))
            .then_with(|| other.marker.cmp(&self.marker))
    }
}

#[derive(Debug, Default, Clone)]
struct Node {
    clip: Option<Arc<AnimClip>>,
    time: f64,
    cycle: i64,
    playback: u64,
    initial: bool,
    rate: f32,
    weight: f32,
    goal_weight: f32,
    goal_time_remaining: f32,
}

impl Node {
    fn reset() -> Self {
        Self {
            rate: 1.0,
            ..Self::default()
        }
    }
}

/// Clip scheduler (iw4L `ClipScheduler`).
#[derive(Debug, Clone)]
pub struct ClipScheduler {
    next_playback: u64,
    nodes: Vec<Node>,
}

impl ClipScheduler {
    /// `node_count` empty nodes at rate 1.
    pub fn new(node_count: usize) -> Self {
        Self {
            next_playback: 1,
            nodes: (0..node_count).map(|_| Node::reset()).collect(),
        }
    }

    /// Number of nodes.
    pub fn node_count(&self) -> usize {
        self.nodes.len()
    }

    /// Puts `clip` in `node` and rewinds it to 0 (weight and rate are kept).
    pub fn set_clip(&mut self, node: usize, clip: Arc<AnimClip>) -> Result<(), ClipSchedulerError> {
        self.node(node)?;
        let playback = self.next_playback;
        self.next_playback = self.next_playback.wrapping_add(1);
        let node = self.node_mut(node)?;
        node.clip = Some(clip);
        node.time = 0.0;
        node.cycle = 0;
        node.playback = playback;
        node.initial = true;
        Ok(())
    }

    /// Empties `node`.
    pub fn clear(&mut self, node: usize) -> Result<(), ClipSchedulerError> {
        *self.node_mut(node)? = Node::reset();
        Ok(())
    }

    /// Sets the clip time of `node`, seconds (negative and non-finite become 0).
    pub fn set_time(&mut self, node: usize, time: f32) -> Result<(), ClipSchedulerError> {
        let node = self.node_mut(node)?;
        node.time = if time.is_finite() {
            f64::from(time.max(0.0))
        } else {
            0.0
        };
        node.initial = false;
        Ok(())
    }

    /// Sets the playback rate of `node` (non-finite becomes 0).
    pub fn set_rate(&mut self, node: usize, rate: f32) -> Result<(), ClipSchedulerError> {
        self.node_mut(node)?.rate = if rate.is_finite() { rate } else { 0.0 };
        Ok(())
    }

    /// Ramps the weight of `node` to `goal_weight` over `goal_time` seconds (0 = at once).
    pub fn set_goal_weight(
        &mut self,
        node: usize,
        goal_weight: f32,
        goal_time: f32,
    ) -> Result<(), ClipSchedulerError> {
        let node = self.node_mut(node)?;
        node.goal_weight = goal_weight.max(0.0);
        node.goal_time_remaining = goal_time.max(0.0);
        if node.goal_time_remaining == 0.0 {
            node.weight = node.goal_weight;
        }
        Ok(())
    }

    /// Advances every node by `dt` seconds and returns the notetracks crossed, earliest first.
    pub fn advance(&mut self, dt: f32) -> ClipAdvance {
        let dt = if dt.is_finite() { dt.max(0.0) } else { 0.0 };
        let mut crossings = BinaryHeap::new();
        let mut total = 0u64;
        for (index, node) in self.nodes.iter_mut().enumerate() {
            if node.weight > 0.0 {
                collect_crossings(node, index, dt, &mut crossings, &mut total);
            }
            advance_goal(node, dt);
        }
        let capacity = usize::try_from(total)
            .unwrap_or(NOTIFY_CAPACITY)
            .min(NOTIFY_CAPACITY);
        let mut notifications = Vec::with_capacity(capacity);
        while notifications.len() < NOTIFY_CAPACITY {
            let Some(mut crossing) = crossings.pop() else {
                break;
            };
            let node = &self.nodes[crossing.node];
            // A crossing is only collected for a node with a clip.
            let Some(name) = node
                .clip
                .as_ref()
                .and_then(|clip| clip.notifies.get(crossing.marker))
                .map(|n| n.name.clone())
            else {
                continue;
            };
            notifications.push(ClipNotification {
                playback: node.playback,
                node: crossing.node,
                cycle: crossing.cycle,
                marker: crossing.marker,
                name,
            });
            crossing.remaining -= 1;
            if crossing.remaining != 0 {
                crossing.cycle = crossing.cycle.saturating_add(crossing.step);
                crossing.offset += crossing.spacing;
                crossings.push(crossing);
            }
        }
        ClipAdvance {
            discarded: total.saturating_sub(notifications.len() as u64),
            notifications,
        }
    }

    /// Nodes with weight > 0 and a clip, in node order.
    pub fn active(&self) -> impl Iterator<Item = ActiveAnim<'_>> {
        self.nodes.iter().enumerate().filter_map(|(node, state)| {
            if state.weight <= 0.0 {
                return None;
            }
            state.clip.as_deref().map(|clip| ActiveAnim {
                node,
                clip,
                time: state.time as f32,
                rate: state.rate,
                weight: state.weight,
            })
        })
    }

    /// Current weight of `node`.
    pub fn weight(&self, node: usize) -> Result<f32, ClipSchedulerError> {
        Ok(self.node(node)?.weight)
    }

    /// Current clip time of `node`, seconds.
    pub fn time(&self, node: usize) -> Result<f32, ClipSchedulerError> {
        Ok(self.node(node)?.time as f32)
    }

    /// Current rate of `node`.
    pub fn rate(&self, node: usize) -> Result<f32, ClipSchedulerError> {
        Ok(self.node(node)?.rate)
    }

    fn node(&self, node: usize) -> Result<&Node, ClipSchedulerError> {
        self.nodes.get(node).ok_or(ClipSchedulerError::BadNode {
            node,
            len: self.nodes.len(),
        })
    }

    fn node_mut(&mut self, node: usize) -> Result<&mut Node, ClipSchedulerError> {
        let len = self.nodes.len();
        self.nodes
            .get_mut(node)
            .ok_or(ClipSchedulerError::BadNode { node, len })
    }
}

/// clip_scheduler.rs:259-336. Exact float comparisons are the upstream logic (a step that does
/// not move the clip neither clears `initial` nor emits notetracks).
#[allow(clippy::float_cmp)]
fn collect_crossings(
    node: &mut Node,
    index: usize,
    dt: f32,
    crossings: &mut BinaryHeap<Crossing>,
    total: &mut u64,
) {
    let Some(clip) = &node.clip else { return };
    let duration = f64::from(clip.duration());
    if !duration.is_finite() || duration <= f64::from(f32::EPSILON) {
        node.time = 0.0;
        return;
    }
    let rate = f64::from(node.rate);
    let old = if clip.looping {
        node.cycle as f64 + node.time / duration
    } else {
        node.time / duration
    };
    let next = old + f64::from(dt) * rate / duration;
    let new = if clip.looping {
        next
    } else {
        next.clamp(0.0, 1.0)
    };
    let initial = node.initial;
    if new != old {
        node.initial = false;
    }
    if clip.looping {
        node.cycle = new.floor() as i64;
        node.time = new.rem_euclid(1.0) * duration;
    } else {
        node.time = new * duration;
    }
    if node.goal_weight <= 0.0 || new == old {
        return;
    }
    for (marker, notify) in clip.notifies.iter().enumerate() {
        if notify.name.is_empty()
            || notify.name.eq_ignore_ascii_case("end")
            || !notify.time.is_finite()
        {
            continue;
        }
        let point = f64::from(notify.time.clamp(0.0, 1.0));
        let (first, last, step) = if new > old {
            let first = if initial && old == 0.0 && point == 0.0 {
                0
            } else {
                ((old - point).floor() as i64).saturating_add(1)
            };
            (first, (new - point).floor() as i64, 1)
        } else {
            (
                ((old - point).ceil() as i64).saturating_sub(1),
                (new - point).ceil() as i64,
                -1,
            )
        };
        if !clip.looping
            && ((step == 1 && (first > 0 || last < 0)) || (step == -1 && (first < 0 || last > 0)))
        {
            continue;
        }
        let (first, last) = if clip.looping { (first, last) } else { (0, 0) };
        if (step == 1 && first > last) || (step == -1 && first < last) {
            continue;
        }
        let count = first.abs_diff(last).saturating_add(1);
        *total = total.saturating_add(count);
        crossings.push(Crossing {
            offset: (first as f64 + point - old) * duration / rate,
            spacing: duration / rate.abs(),
            node: index,
            marker,
            cycle: first,
            step,
            remaining: count,
        });
    }
}

/// clip_scheduler.rs:338-349.
fn advance_goal(node: &mut Node, dt: f32) {
    if node.goal_time_remaining <= 0.0 {
        return;
    }
    let step = dt.min(node.goal_time_remaining);
    node.weight += (node.goal_weight - node.weight) * step / node.goal_time_remaining;
    node.goal_time_remaining -= step;
    if node.goal_time_remaining <= f32::EPSILON {
        node.weight = node.goal_weight;
        node.goal_time_remaining = 0.0;
    }
}

#[cfg(test)]
#[allow(clippy::unwrap_used, clippy::float_cmp)]
mod tests {
    use super::*;
    use xmodel_runtime::{ClipNotify, Translation};

    fn clip(looping: bool, notifies: &[(&str, f32)]) -> Arc<AnimClip> {
        Arc::new(AnimClip {
            name: "c".into(),
            framerate: 10.0,
            numframes: 10,
            looping,
            tracks: vec![],
            notifies: notifies
                .iter()
                .map(|&(name, time)| ClipNotify {
                    name: name.into(),
                    time,
                })
                .collect(),
            delta_translation: Translation::Default,
        })
    }

    #[test]
    fn notifies_fire_once_in_order_and_clamp_at_end() {
        let mut s = ClipScheduler::new(2);
        s.set_clip(0, clip(false, &[("b", 0.5), ("a", 0.2)]))
            .unwrap();
        s.set_goal_weight(0, 1.0, 0.0).unwrap();
        let names: Vec<_> = s
            .advance(0.6)
            .notifications
            .into_iter()
            .map(|n| n.name)
            .collect();
        assert_eq!(names, ["a", "b"]);
        assert!(s.advance(5.0).notifications.is_empty());
        assert!((s.time(0).unwrap() - 1.0).abs() < 1e-6);
    }

    #[test]
    fn looping_wraps_and_repeats_notifies() {
        let mut s = ClipScheduler::new(1);
        s.set_clip(0, clip(true, &[("tick", 0.5)])).unwrap();
        s.set_goal_weight(0, 1.0, 0.0).unwrap();
        let adv = s.advance(2.6);
        assert_eq!(adv.notifications.len(), 3);
        assert!((s.time(0).unwrap() - 0.6).abs() < 1e-5);
    }

    #[test]
    fn weight_ramps_to_goal() {
        let mut s = ClipScheduler::new(1);
        s.set_clip(0, clip(false, &[])).unwrap();
        s.set_goal_weight(0, 1.0, 0.5).unwrap();
        s.advance(0.25);
        assert!((s.weight(0).unwrap() - 0.5).abs() < 1e-6);
        s.advance(0.5);
        assert_eq!(s.weight(0).unwrap(), 1.0);
        assert!(s.set_rate(3, 1.0).is_err());
    }
}
