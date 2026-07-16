use pourpoint_core::algo::coord::GeoCoord;
use pourpoint_core::resolve_outlet;
use pourpoint_core::session::DatasetSession;
use pourpoint_core::testutil::{DatasetBuilder, TestCatchment, TestSnapGeometry, TestSnapTarget};
use pourpoint_core::{
    BestEffortSkipReason, DelineationOptions, Engine, RefinementOutcome, RefinementProvenance,
    RefinementStrategyName, ResolutionMethod, ResolverConfig,
};

#[test]
fn hydrobasins_snap_delineates_without_d8_refinement() {
    let (_fixture, path) = DatasetBuilder::new(2)
        .with_custom_catchments(vec![
            TestCatchment {
                id: 4_120_057_290,
                area_km2: 1.0,
                up_area_km2: Some(1.0),
                polygon: (85.30, 27.70, 85.31, 27.71),
            },
            TestCatchment {
                id: 4_120_057_300,
                area_km2: 1.0,
                up_area_km2: Some(2.0),
                polygon: (85.31, 27.70, 85.32, 27.71),
            },
        ])
        .with_custom_snap_targets(vec![TestSnapTarget {
            id: 1,
            catchment_id: 4_120_057_290,
            weight: 1.0,
            is_mainstem: true,
            geometry: TestSnapGeometry::LineString(85.306, 27.705, 85.314, 27.705),
        }])
        .build();
    let outlet = GeoCoord::new(85.312, 27.705);

    let resolver_session =
        DatasetSession::open_path(&path).expect("HydroBASINS snap fixture should open");
    let resolved = resolve_outlet(&resolver_session, outlet, &ResolverConfig::default())
        .expect("outlet on the HydroRIVERS LineString should resolve");
    assert!(
        matches!(resolved.method, ResolutionMethod::Snap { .. }),
        "declared snap data must dispatch to Snap, got {:?}",
        resolved.method
    );

    let engine_session =
        DatasetSession::open_path(&path).expect("HydroBASINS snap fixture should reopen");
    let engine = Engine::builder(engine_session).build();
    let delineation = engine
        .delineate(outlet, &DelineationOptions::default())
        .expect("snap-resolved HydroBASINS delineation should succeed");

    assert!(
        delineation.area_km2().as_f64() > 0.0,
        "delineated watershed area must be positive"
    );
    assert_eq!(
        delineation.refinement(),
        &RefinementOutcome::BestEffortSkipped {
            provenance: RefinementProvenance::BestEffortSkipped {
                strategy: RefinementStrategyName::BestEffortD8IfPresent,
                why: BestEffortSkipReason::NoD8AuxDeclared,
            },
        }
    );
}
