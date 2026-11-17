use super::*;
fn point() -> TargetPoint {
    TargetPoint::from_values([0.; 6])
}

#[test]
fn regional_dominance_absorbs_an_isolated_biome_without_erasing_habitat_niches() {
    let mut samples = vec![(Habitat::Temperate, "forest", point()); 8];
    samples.push((Habitat::Temperate, "tiny_redwood", point()));
    let p = Province::census(42, [-3, 2], &samples);
    assert_eq!(p.choose(Habitat::Temperate, point()), Some("forest"));
    samples.reverse();
    assert_eq!(
        Province::census(42, [-3, 2], &samples).choose(Habitat::Temperate, point()),
        Some("forest")
    );
    assert_eq!(p.choose(Habitat::Highland, point()), None);
    assert_eq!(p.choose(Habitat::Wetland, point()), None);
}
#[test]
fn a_climate_family_blip_can_merge_but_deserts_and_snow_cannot() {
    let mut samples = vec![(Habitat::Temperate, "forest", point()); 7];
    samples.extend([(Habitat::Cold, "snow", point()); 2]);
    let p = Province::census(8, [0, 0], &samples);
    assert_eq!(p.choose(Habitat::Cold, point()), Some("forest"));
    let mut frozen = point();
    frozen.temperature = -10000;
    assert_eq!(p.choose(Habitat::Cold, frozen), None);
    let mut samples = vec![(Habitat::Arid, "desert", point()); 5];
    samples.extend([(Habitat::Cold, "snow", point()); 4]);
    let p = Province::census(8, [0, 0], &samples);
    assert_eq!(p.choose(Habitat::Cold, point()), Some("snow"));
    assert_eq!(p.choose(Habitat::Arid, point()), Some("desert"));
}
#[test]
fn protected_samples_never_supply_a_surface_province() {
    let p = Province::census(1, [0, 0], &[(Habitat::Preserve, "cave", point()); 9]);
    assert!(p.choices.iter().all(Option::is_none));
    assert_eq!(p.choose(Habitat::Temperate, point()), None);
}
#[test]
fn jittered_warped_sites_have_wide_cores_on_both_sides_of_zero() {
    for seed in [0, 51, -987654, 5191320528283946732] {
        let r = SurfaceRegions::new(seed, 192);
        for z in -8..8 {
            for x in -8..8 {
                let q = [x * r.span + r.span / 2, z * r.span + r.span / 2];
                let site = r.sites(q[0], q[1])[0];
                for dz in [-r.span / 8, 0, r.span / 8] {
                    for dx in [-r.span / 8, 0, r.span / 8] {
                        assert_eq!(
                            r.sites(site.position[0] + dx, site.position[1] + dz)[0].cell,
                            site.cell
                        );
                    }
                }
            }
        }
    }
}

#[test]
fn extreme_quart_coordinates_do_not_overflow_region_distances() {
    let r = SurfaceRegions::new(42, 192);
    for (x, z) in [
        (i32::MIN, i32::MIN),
        (i32::MAX, i32::MAX),
        (i32::MIN, i32::MAX),
    ] {
        let a = r.sites(x, z);
        let b = r.sites(x, z);
        assert_eq!(a[0].cell, b[0].cell);
        assert!(a.iter().all(|s| s.distance >= 0));
    }
}
