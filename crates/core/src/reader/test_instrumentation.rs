//! reader_session_measurement : OwnerThread × ReaderSessionEvent* → ReaderSessionMeasurements
//!
//! Events are recorded only while the current OS thread owns an explicit cfg(test)-only scope.

use std::cell::{Cell, RefCell};
use std::collections::HashMap;
use std::marker::PhantomData;
use std::rc::Rc;

use hfx::UnitId;

#[derive(Default)]
struct ReaderSessionMeasurements {
    catchment_geometry_decode_counts: HashMap<(String, UnitId), usize>,
    catchment_id_level_scans: usize,
    catchment_id_only_scans: usize,
    catchment_id_level_in_flight: usize,
    catchment_id_level_max_in_flight: usize,
    snap_membership_rows: usize,
    snap_geometry_decode_rows: usize,
    snap_membership_in_flight: usize,
    snap_membership_max_in_flight: usize,
    snap_validation_scans: usize,
}

struct ActiveMeasurement {
    generation: u64,
    values: ReaderSessionMeasurements,
}

thread_local! {
    static ACTIVE: RefCell<Option<ActiveMeasurement>> = const { RefCell::new(None) };
    static NEXT_GENERATION: Cell<u64> = const { Cell::new(1) };
}

fn with_active<R>(read: impl FnOnce(&Option<ActiveMeasurement>) -> R) -> R {
    ACTIVE.with(|active| read(&active.borrow()))
}

fn with_active_mut<R>(update: impl FnOnce(&mut Option<ActiveMeasurement>) -> R) -> R {
    ACTIVE.with(|active| update(&mut active.borrow_mut()))
}

#[derive(Clone, Copy)]
struct OwnerToken {
    generation: u64,
}

/// Owns reader/session measurements on the thread that creates it.
pub(crate) struct ReaderSessionMeasurementScope {
    generation: u64,
    _not_send: PhantomData<Rc<()>>,
}

impl ReaderSessionMeasurementScope {
    /// Install a fresh measurement for the current thread.
    pub(crate) fn enter() -> Self {
        let generation = NEXT_GENERATION.with(|next| {
            let generation = next.get();
            next.set(
                generation
                    .checked_add(1)
                    .expect("measurement generation overflow"),
            );
            generation
        });
        with_active_mut(|active| {
            assert!(
                active.is_none(),
                "reader/session measurement scope already active on this thread"
            );
            *active = Some(ActiveMeasurement {
                generation,
                values: ReaderSessionMeasurements::default(),
            });
        });
        Self {
            generation,
            _not_send: PhantomData,
        }
    }
}

impl Drop for ReaderSessionMeasurementScope {
    fn drop(&mut self) {
        with_active_mut(|active| {
            let current = active
                .as_ref()
                .expect("reader/session measurement scope missing on drop");
            assert_eq!(
                current.generation, self.generation,
                "reader/session measurement scope generation changed before drop"
            );
            assert_eq!(
                current.values.catchment_id_level_in_flight, 0,
                "catchment ID/level reads still in flight when measurement scope dropped"
            );
            assert_eq!(
                current.values.snap_membership_in_flight, 0,
                "snap membership reads still in flight when measurement scope dropped"
            );
            *active = None;
        });
    }
}

fn record(update: impl FnOnce(&mut ReaderSessionMeasurements)) {
    with_active_mut(|active| {
        if let Some(active) = active.as_mut() {
            update(&mut active.values);
        }
    });
}

fn read(value: impl FnOnce(&ReaderSessionMeasurements) -> usize) -> usize {
    with_active(|active| active.as_ref().map_or(0, |active| value(&active.values)))
}

pub(crate) fn record_catchment_geometry_decode(path: &str, unit_id: UnitId) {
    record(|values| {
        *values
            .catchment_geometry_decode_counts
            .entry((path.to_owned(), unit_id))
            .or_default() += 1
    });
}

pub(crate) fn catchment_geometry_decode_rows() -> usize {
    read(|values| values.catchment_geometry_decode_counts.values().sum())
}

pub(crate) fn catchment_geometry_decode_count(path: &str, unit_id: UnitId) -> usize {
    with_active(|active| {
        active
            .as_ref()
            .and_then(|active| {
                active
                    .values
                    .catchment_geometry_decode_counts
                    .get(&(path.to_owned(), unit_id))
                    .copied()
            })
            .unwrap_or_default()
    })
}

pub(crate) fn record_catchment_id_level_scan() {
    record(|values| values.catchment_id_level_scans += 1);
}

pub(crate) fn record_catchment_id_only_scan() {
    record(|values| values.catchment_id_only_scans += 1);
}

pub(crate) fn catchment_id_level_scan_count() -> usize {
    read(|values| values.catchment_id_level_scans)
}

pub(crate) fn catchment_id_only_scan_count() -> usize {
    read(|values| values.catchment_id_only_scans)
}

pub(crate) fn catchment_id_level_max_in_flight() -> usize {
    read(|values| values.catchment_id_level_max_in_flight)
}

pub(crate) struct CatchmentIdLevelInFlightForTest {
    owner: Option<OwnerToken>,
    _not_send: PhantomData<Rc<()>>,
}

impl CatchmentIdLevelInFlightForTest {
    pub(crate) fn enter() -> Self {
        let owner = with_active_mut(|active| {
            active.as_mut().map(|active| {
                active.values.catchment_id_level_in_flight = active
                    .values
                    .catchment_id_level_in_flight
                    .checked_add(1)
                    .expect("catchment ID/level in-flight overflow");
                active.values.catchment_id_level_max_in_flight = active
                    .values
                    .catchment_id_level_max_in_flight
                    .max(active.values.catchment_id_level_in_flight);
                OwnerToken {
                    generation: active.generation,
                }
            })
        });
        Self {
            owner,
            _not_send: PhantomData,
        }
    }
}

impl Drop for CatchmentIdLevelInFlightForTest {
    fn drop(&mut self) {
        let Some(owner) = self.owner else { return };
        with_active_mut(|active| {
            let active = active
                .as_mut()
                .expect("catchment ID/level in-flight owner missing on drop");
            assert_eq!(
                active.generation, owner.generation,
                "catchment ID/level in-flight owner generation changed before drop"
            );
            active.values.catchment_id_level_in_flight = active
                .values
                .catchment_id_level_in_flight
                .checked_sub(1)
                .expect("catchment ID/level in-flight underflow on drop");
        });
    }
}

pub(crate) fn record_snap_membership_rows(rows: usize) {
    record(|values| values.snap_membership_rows += rows);
}
pub(crate) fn snap_membership_rows() -> usize {
    read(|values| values.snap_membership_rows)
}
pub(crate) fn record_snap_geometry_decode_row() {
    record(|values| values.snap_geometry_decode_rows += 1);
}
pub(crate) fn snap_geometry_decode_rows() -> usize {
    read(|values| values.snap_geometry_decode_rows)
}
pub(crate) fn snap_membership_max_in_flight() -> usize {
    read(|values| values.snap_membership_max_in_flight)
}

pub(crate) struct SnapMembershipInFlightForTest {
    owner: Option<OwnerToken>,
    _not_send: PhantomData<Rc<()>>,
}

impl SnapMembershipInFlightForTest {
    pub(crate) fn enter() -> Self {
        let owner = with_active_mut(|active| {
            active.as_mut().map(|active| {
                active.values.snap_membership_in_flight = active
                    .values
                    .snap_membership_in_flight
                    .checked_add(1)
                    .expect("snap membership in-flight overflow");
                active.values.snap_membership_max_in_flight = active
                    .values
                    .snap_membership_max_in_flight
                    .max(active.values.snap_membership_in_flight);
                OwnerToken {
                    generation: active.generation,
                }
            })
        });
        Self {
            owner,
            _not_send: PhantomData,
        }
    }
}

impl Drop for SnapMembershipInFlightForTest {
    fn drop(&mut self) {
        let Some(owner) = self.owner else { return };
        with_active_mut(|active| {
            let active = active
                .as_mut()
                .expect("snap membership in-flight owner missing on drop");
            assert_eq!(
                active.generation, owner.generation,
                "snap membership in-flight owner generation changed before drop"
            );
            active.values.snap_membership_in_flight = active
                .values
                .snap_membership_in_flight
                .checked_sub(1)
                .expect("snap membership in-flight underflow on drop");
        });
    }
}

pub(crate) fn record_snap_validation_scan() {
    record(|values| values.snap_validation_scans += 1);
}
pub(crate) fn snap_validation_scan_count() -> usize {
    read(|values| values.snap_validation_scans)
}
