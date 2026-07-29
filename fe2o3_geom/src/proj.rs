//! Map projections: turning a position on a sphere into a position on a page.
//!
//! A projection here is *forward* only — geodetic latitude and longitude in to plane
//! coordinates out — because that is what drawing a map needs. The plane is right-handed
//! with `y` increasing northward, which is the convention every projection reference uses;
//! a caller drawing into a screen or an SVG negates `y` itself, since only the caller knows
//! which way its own axis runs.
//!
//! Scale is the caller's too. Every function takes a sphere radius, and the unit-sphere
//! constants are exposed so that a caller wanting a map of a given width can work out the
//! radius that produces it ([`equal_earth_radius_for_half_width`]).
//!
//! Equal Earth draws the whole world at once; orthographic draws the half of it a viewer is
//! looking at, as a globe. Further projections belong beside them in this module rather than
//! in a module of their own.

use crate::planar::Pt;

// ---------------------------------------------------------------------------------------------
// Equal Earth
// ---------------------------------------------------------------------------------------------

/// First polynomial coefficient of the Equal Earth projection.
///
/// The four coefficients are those published by Šavrič, Patterson and Jenny, *The Equal
/// Earth map projection*, International Journal of Geographical Information Science 33(3),
/// 2019, and are the same values PROJ carries for `+proj=eqearth`.
pub const EQUAL_EARTH_A1: f64 = 1.340264;

/// Second polynomial coefficient of the Equal Earth projection.
pub const EQUAL_EARTH_A2: f64 = -0.081106;

/// Third polynomial coefficient of the Equal Earth projection.
pub const EQUAL_EARTH_A3: f64 = 0.000893;

/// Fourth polynomial coefficient of the Equal Earth projection.
pub const EQUAL_EARTH_A4: f64 = 0.003796;

/// Half the width of the projected world on a unit sphere.
///
/// The value of `x` at the equator on the antimeridian, which is where the map is widest.
pub const EQUAL_EARTH_HALF_WIDTH: f64 = 2.706_629_983_696_074_3;

/// Half the height of the projected world on a unit sphere.
///
/// The value of `y` at a pole. The world is therefore 2.0546 times as wide as it is tall.
pub const EQUAL_EARTH_HALF_HEIGHT: f64 = 1.317_362_759_157_413;

/// Projects a position onto the Equal Earth plane.
///
/// Equal Earth is pseudocylindrical and equal-area: every square metre of ground occupies
/// the same area on the page wherever it is, which is what makes it honest about how much
/// of the world a continent is. Parallels are straight and evenly spaced enough to read,
/// meridians are curved, and the shape of the land is close to what a person expects.
///
/// # Arguments
/// * `lat` - Latitude in signed decimal degrees, positive north.
/// * `lng` - Longitude in signed decimal degrees, positive east.
/// * `lon_0` - The central meridian, in degrees. Zero for Greenwich down the middle.
/// * `radius` - The radius of the sphere, in whatever units the result should come out in.
///
/// # Returns
/// The projected point, `x` eastward and `y` northward from the map's centre.
///
/// Latitude is clamped to the poles and longitude wrapped into a half turn either side of
/// `lon_0`, so a fix arriving one part in a billion outside its range projects rather than
/// producing a not-a-number. A non-finite argument yields a non-finite result: the caller
/// is the one that knows whether that is a hole in its data or an error.
pub fn equal_earth(lat: f64, lng: f64, lon_0: f64, radius: f64) -> Pt {
    let phi = lat.clamp(-90.0, 90.0).to_radians();
    let lam = wrap_half_turn(lng - lon_0).to_radians();

    // The parametric latitude, on which the whole projection is a polynomial.
    let theta = ((3.0f64).sqrt() / 2.0 * phi.sin()).clamp(-1.0, 1.0).asin();
    let t2 = theta * theta;
    let t3 = t2 * theta;
    let t6 = t3 * t3;
    let t7 = t6 * theta;
    let t8 = t6 * t2;
    let t9 = t8 * theta;

    // The denominator of x is dy/dθ, which is what makes the projection equal-area.
    let dy = 9.0 * EQUAL_EARTH_A4 * t8
        + 7.0 * EQUAL_EARTH_A3 * t6
        + 3.0 * EQUAL_EARTH_A2 * t2
        + EQUAL_EARTH_A1;

    let x = 2.0 * (3.0f64).sqrt() * lam * theta.cos() / (3.0 * dy);
    let y = EQUAL_EARTH_A4 * t9
        + EQUAL_EARTH_A3 * t7
        + EQUAL_EARTH_A2 * t3
        + EQUAL_EARTH_A1 * theta;

    Pt::new(radius * x, radius * y)
}

/// Projects a position onto the Equal Earth plane of a unit sphere, Greenwich centred.
///
/// The bare form, for a caller that will scale the result itself.
pub fn equal_earth_unit(lat: f64, lng: f64) -> Pt {
    equal_earth(lat, lng, 0.0, 1.0)
}

/// The sphere radius that makes an Equal Earth map `2 * half_width` across.
///
/// A map is usually specified by the box it has to fill rather than by the size of the
/// world it draws, and this converts one into the other. The height that follows is
/// `half_width * EQUAL_EARTH_HALF_HEIGHT / EQUAL_EARTH_HALF_WIDTH`, or very nearly half
/// the width.
pub fn equal_earth_radius_for_half_width(half_width: f64) -> f64 {
    half_width / EQUAL_EARTH_HALF_WIDTH
}

// ---------------------------------------------------------------------------------------------
// Orthographic
// ---------------------------------------------------------------------------------------------

/// Half the width of the orthographic projection on a unit sphere.
///
/// The projection is the sphere seen from infinitely far away, so the map is the sphere's own
/// disc and its half width is the radius. A caller wanting a globe `2 * half_width` across
/// therefore passes `half_width` as the radius; there is no conversion to do.
pub const ORTHOGRAPHIC_HALF_WIDTH: f64 = 1.0;

/// Projects a position onto the orthographic plane, as seen from above `lat_0`, `lon_0`.
///
/// The orthographic projection is what a sphere looks like from far enough away that the rays
/// arrive parallel: the near hemisphere fills a disc, foreshortened towards the rim, and the
/// far hemisphere is behind it. It is neither equal-area nor conformal, and it is the only
/// projection that does not have to choose, because it is not flattening the world at all —
/// it is drawing the object. That makes it the honest one for a globe a viewer turns.
///
/// # Arguments
/// * `lat` - Latitude in signed decimal degrees, positive north.
/// * `lng` - Longitude in signed decimal degrees, positive east.
/// * `lat_0` - The latitude the viewer is above, in degrees.
/// * `lon_0` - The longitude the viewer is above, in degrees.
/// * `radius` - The radius of the sphere, in whatever units the result should come out in.
///
/// # Returns
/// The projected point, `x` rightward and `y` upward from the centre of the disc.
///
/// **A position on the far hemisphere still projects**, onto the point of the near hemisphere
/// directly in front of it, because the formula cannot tell them apart. Ask
/// [`orthographic_cos_c`] which side of the globe a position is on; it is separate so that a
/// caller clipping a coastline can interpolate along an edge to the horizon, where that
/// cosine is zero, rather than being handed a hole.
pub fn orthographic(lat: f64, lng: f64, lat_0: f64, lon_0: f64, radius: f64) -> Pt {
    let phi = lat.clamp(-90.0, 90.0).to_radians();
    let phi_0 = lat_0.clamp(-90.0, 90.0).to_radians();
    let lam = wrap_half_turn(lng - lon_0).to_radians();

    let x = phi.cos() * lam.sin();
    let y = phi_0.cos() * phi.sin() - phi_0.sin() * phi.cos() * lam.cos();

    Pt::new(radius * x, radius * y)
}

/// The cosine of the angle between a position and the centre of an orthographic projection.
///
/// One where the position is under the viewer, zero on the horizon, and negative on the far
/// side of the globe. A caller drawing a coastline keeps the vertices where this is positive
/// and finds the horizon crossing by interpolating an edge to where it is zero.
///
/// # Arguments
/// * `lat` - Latitude in signed decimal degrees, positive north.
/// * `lng` - Longitude in signed decimal degrees, positive east.
/// * `lat_0` - The latitude the viewer is above, in degrees.
/// * `lon_0` - The longitude the viewer is above, in degrees.
pub fn orthographic_cos_c(lat: f64, lng: f64, lat_0: f64, lon_0: f64) -> f64 {
    let phi = lat.clamp(-90.0, 90.0).to_radians();
    let phi_0 = lat_0.clamp(-90.0, 90.0).to_radians();
    let lam = wrap_half_turn(lng - lon_0).to_radians();
    phi_0.sin() * phi.sin() + phi_0.cos() * phi.cos() * lam.cos()
}

/// Brings an angle in degrees into the half turn either side of zero.
///
/// A longitude difference of 190° east is 170° west, and a map has to draw it there.
fn wrap_half_turn(deg: f64) -> f64 {
    if !deg.is_finite() {
        return deg;
    }
    let full = 360.0;
    let mut d = deg % full;
    if d > 180.0 {
        d -= full;
    } else if d < -180.0 {
        d += full;
    }
    d
}

#[cfg(test)]
mod tests {
    use super::*;

    use oxedyne_fe2o3_core::prelude::*;

    /// How close a projected coordinate has to be to the oracle's, on a unit sphere.
    ///
    /// The oracle is printed to twelve decimal places, so this is loose enough to absorb its
    /// own rounding and tight enough that a wrong coefficient could not pass.
    const TOL: f64 = 1.0e-11;

    /// Positions and their projections on a unit sphere, Greenwich centred.
    ///
    /// The expected values come from PROJ 9.7.1, an implementation outside this crate:
    ///
    /// ```text
    /// proj -f "%.12f" +proj=eqearth +R=1 +lon_0=0
    /// ```
    ///
    /// PROJ is the reference implementation the projection's authors' work was folded into,
    /// and it is not derived from anything here.
    const ORACLE: [(f64, f64, f64, f64); 18] = [
        // Perth.
        (-31.9535,  115.8571,   1.614621210487, -0.629372441721),
        // London.
        ( 51.5074,   -0.1278,  -0.001565514410,  0.965105647318),
        // New York.
        ( 40.7128,  -74.0060,  -0.981861449986,  0.787064220049),
        // Tokyo.
        ( 35.6895,  139.6917,   1.909462941700,  0.697844715778),
        // Cape Town.
        (-33.9189,   18.4233,   0.254226330215, -0.665601683690),
        // Buenos Aires.
        (-34.6037,  -58.3816,  -0.802723755324, -0.678117876992),
        // Sydney.
        (-33.8688,  151.2093,   2.087106438222, -0.664683776716),
        // Honolulu.
        ( 21.3069, -157.8580,  -2.295788127853,  0.426387151159),
        // The origin, and the widest and tallest points of the map.
        (  0.0,        0.0,     0.0,             0.0),
        (  0.0,      180.0,     2.706629983696,  0.0),
        (  0.0,     -180.0,    -2.706629983696,  0.0),
        ( 90.0,        0.0,     0.0,             1.317362759157),
        (-90.0,        0.0,     0.0,            -1.317362759157),
        // Round numbers in all four quadrants.
        ( 45.0,       90.0,     1.159854499103,  0.860231085522),
        (-45.0,      -90.0,    -1.159854499103, -0.860231085522),
        ( 60.0,       30.0,     0.339843347929,  1.088300835505),
        (-60.0,      120.0,     1.359373391714, -1.088300835505),
        // A small angle, where the polynomial's high terms contribute nothing.
        (  1.0,        2.0,     0.030071478499,  0.020257546047),
    ];

    #[test]
    fn test_equal_earth_agrees_with_proj_00() -> Outcome<()> {
        for (lat, lng, x, y) in ORACLE {
            let p = equal_earth_unit(lat, lng);
            let near_x = (p.x - x).abs() < TOL;
            let near_y = (p.y - y).abs() < TOL;
            req!(near_x, true, "{}, {} came out at x = {:.12}, wanted {:.12}.", lat, lng, p.x, x);
            req!(near_y, true, "{}, {} came out at y = {:.12}, wanted {:.12}.", lat, lng, p.y, y);
        }
        Ok(())
    }

    #[test]
    fn test_the_stated_extremes_are_the_extremes_01() -> Outcome<()> {
        let east = equal_earth_unit(0.0, 180.0);
        let near = (east.x - EQUAL_EARTH_HALF_WIDTH).abs() < TOL;
        req!(near, true, "The map is {} wide, not {}.", east.x, EQUAL_EARTH_HALF_WIDTH);
        let north = equal_earth_unit(90.0, 0.0);
        let near = (north.y - EQUAL_EARTH_HALF_HEIGHT).abs() < TOL;
        req!(near, true, "The map is {} tall, not {}.", north.y, EQUAL_EARTH_HALF_HEIGHT);
        // No point escapes the box those two describe.
        let mut lat = -90.0;
        while lat <= 90.0 {
            let mut lng = -180.0;
            while lng <= 180.0 {
                let p = equal_earth_unit(lat, lng);
                let in_x = p.x.abs() <= EQUAL_EARTH_HALF_WIDTH + TOL;
                let in_y = p.y.abs() <= EQUAL_EARTH_HALF_HEIGHT + TOL;
                req!(in_x, true, "x escaped at {}, {}.", lat, lng);
                req!(in_y, true, "y escaped at {}, {}.", lat, lng);
                lng += 3.0;
            }
            lat += 3.0;
        }
        Ok(())
    }

    #[test]
    fn test_the_projection_is_symmetric_02() -> Outcome<()> {
        // A pseudocylindrical projection is symmetric about both axes, so the same
        // latitude north and south is the same height, and east and west the same width.
        for (lat, lng) in [(31.9535, 115.8571), (12.0, 5.0), (78.0, 179.0)] {
            let a = equal_earth_unit(lat, lng);
            let b = equal_earth_unit(-lat, lng);
            let c = equal_earth_unit(lat, -lng);
            let same_x = (a.x - b.x).abs() < TOL;
            let flip_y = (a.y + b.y).abs() < TOL;
            let flip_x = (a.x + c.x).abs() < TOL;
            let same_y = (a.y - c.y).abs() < TOL;
            req!(same_x, true, "x differed across the equator at {}.", lat);
            req!(flip_y, true, "y was not mirrored across the equator at {}.", lat);
            req!(flip_x, true, "x was not mirrored across Greenwich at {}.", lng);
            req!(same_y, true, "y differed across Greenwich at {}.", lng);
        }
        // A parallel is straight: every longitude on it has the same y.
        let y = equal_earth_unit(20.0, 0.0).y;
        for lng in [-170.0, -60.0, 45.0, 179.9] {
            let p = equal_earth_unit(20.0, lng);
            let level = (p.y - y).abs() < TOL;
            req!(level, true, "The 20° parallel bent at {}.", lng);
        }
        Ok(())
    }

    #[test]
    fn test_a_central_meridian_moves_the_map_and_not_its_shape_03() -> Outcome<()> {
        // Recentring on 150° puts Sydney where Greenwich centring puts 1.209° east.
        let a = equal_earth(-33.8688, 151.2093, 150.0, 1.0);
        let b = equal_earth(-33.8688, 1.2093, 0.0, 1.0);
        let same_x = (a.x - b.x).abs() < TOL;
        let same_y = (a.y - b.y).abs() < TOL;
        req!(same_x, true, "Recentring moved x to {} rather than {}.", a.x, b.x);
        req!(same_y, true, "Recentring moved y at all: {} against {}.", a.y, b.y);
        // And a longitude that wraps past the antimeridian lands on the far side rather
        // than off the map.
        let west = equal_earth(0.0, -170.0, 20.0, 1.0);
        let wrapped = west.x > 0.0;
        req!(wrapped, true, "170° west of a map centred on 20° east came out at {}.", west.x);
        Ok(())
    }

    #[test]
    fn test_a_radius_scales_the_map_uniformly_04() -> Outcome<()> {
        let r = equal_earth_radius_for_half_width(180.0);
        let east = equal_earth(0.0, 180.0, 0.0, r);
        let wide = (east.x - 180.0).abs() < 1.0e-9;
        req!(wide, true, "The map came out {} wide.", east.x);
        let north = equal_earth(90.0, 0.0, 0.0, r);
        let want = 180.0 * EQUAL_EARTH_HALF_HEIGHT / EQUAL_EARTH_HALF_WIDTH;
        let tall = (north.y - want).abs() < 1.0e-9;
        req!(tall, true, "The map came out {} tall.", north.y);
        // Scaling is uniform, or the projection would stop being equal-area.
        let one = equal_earth_unit(-31.9535, 115.8571);
        let big = equal_earth(-31.9535, 115.8571, 0.0, r);
        let scaled_x = (big.x - one.x * r).abs() < 1.0e-9;
        let scaled_y = (big.y - one.y * r).abs() < 1.0e-9;
        req!(scaled_x, true, "x did not scale by the radius.");
        req!(scaled_y, true, "y did not scale by the radius.");
        Ok(())
    }

    /// Positions and their orthographic projections on a unit sphere centred on the origin.
    ///
    /// From PROJ 9.7.1, an implementation outside this crate:
    ///
    /// ```text
    /// proj -f "%.12f" +proj=ortho +R=1 +lat_0=0 +lon_0=0
    /// ```
    ///
    /// Every position here is on the near hemisphere, because PROJ answers `*` for the far
    /// one rather than a coordinate; that refusal is what
    /// [`test_the_far_side_of_the_globe_is_known_from_the_near_07`] checks against.
    const ORTHO_ORACLE: [(f64, f64, f64, f64); 11] = [
        // The centre itself.
        (  0.0,       0.0,     0.0,             0.0),
        // London, Cape Town, Buenos Aires, New York: all within a quarter turn of Greenwich.
        ( 51.5074,   -0.1278, -0.001388311442,  0.782688550807),
        (-33.9189,   18.4233,  0.262254675949, -0.558018872482),
        (-34.6037,  -58.3816, -0.700917639761, -0.567896899696),
        ( 40.7128,  -74.0060, -0.728647317901,  0.652267756393),
        // The rim, a tenth of a degree short of it.
        (  0.0,      89.9,     0.999998476913,  0.0),
        // Both poles, which sit at the top and bottom of the disc from the equator.
        ( 90.0,       0.0,     0.0,             1.0),
        (-90.0,       0.0,     0.0,            -1.0),
        // Round numbers, and a small angle where the foreshortening is negligible.
        (  1.0,       2.0,     0.034894181340,  0.017452406437),
        ( 60.0,      30.0,     0.250000000000,  0.866025403784),
        (-45.0,     -90.0,    -0.707106781187, -0.707106781187),
    ];

    /// The same, seen from above Perth, so the centre is oblique in both coordinates.
    ///
    /// ```text
    /// proj -f "%.12f" +proj=ortho +R=1 +lat_0=-31.9535 +lon_0=115.8571
    /// ```
    const ORTHO_ORACLE_PERTH: [(f64, f64, f64, f64); 8] = [
        // Perth, which is the centre.
        (-31.9535,  115.8571,  0.0,             0.0),
        // Sydney, Tokyo, Kuala Lumpur, Auckland, Suva.
        (-33.8688,  151.2093,  0.480421544470, -0.114447989448),
        ( 35.6895,  139.6917,  0.328204336296,  0.888173527162),
        (  3.1390,  101.6869, -0.244435840733,  0.558819302390),
        (-36.8485,  174.7633,  0.685250212424, -0.290118911137),
        (-18.1416,  178.4419,  0.843565957319, -0.032624197369),
        // Fremantle, sixteen kilometres away and barely off the centre.
        (-32.0569,  115.7439, -0.001674457753, -0.001805544881),
        // The south pole, which from this latitude sits low on the disc and not on its rim.
        (-90.0,       0.0,     0.0,            -0.848477887693),
    ];

    #[test]
    fn test_orthographic_agrees_with_proj_06() -> Outcome<()> {
        for (lat, lng, x, y) in ORTHO_ORACLE {
            let p = orthographic(lat, lng, 0.0, 0.0, 1.0);
            let near_x = (p.x - x).abs() < TOL;
            let near_y = (p.y - y).abs() < TOL;
            req!(near_x, true, "{}, {} came out at x = {:.12}, wanted {:.12}.", lat, lng, p.x, x);
            req!(near_y, true, "{}, {} came out at y = {:.12}, wanted {:.12}.", lat, lng, p.y, y);
        }
        for (lat, lng, x, y) in ORTHO_ORACLE_PERTH {
            let p = orthographic(lat, lng, -31.9535, 115.8571, 1.0);
            let near_x = (p.x - x).abs() < TOL;
            let near_y = (p.y - y).abs() < TOL;
            req!(near_x, true, "{}, {} came out at x = {:.12}, wanted {:.12}.", lat, lng, p.x, x);
            req!(near_y, true, "{}, {} came out at y = {:.12}, wanted {:.12}.", lat, lng, p.y, y);
        }
        Ok(())
    }

    #[test]
    fn test_the_far_side_of_the_globe_is_known_from_the_near_07() -> Outcome<()> {
        // Perth is a quarter turn and more from Greenwich, so PROJ answers `*` for it on a
        // Greenwich-centred globe. The cosine is what says so here.
        let behind = orthographic_cos_c(-31.9535, 115.8571, 0.0, 0.0);
        let far = behind < 0.0;
        req!(far, true, "Perth came out in front of Greenwich, at {}.", behind);
        // The centre is directly under the viewer, and its antipode directly behind.
        let under = orthographic_cos_c(-31.9535, 115.8571, -31.9535, 115.8571);
        let full = (under - 1.0).abs() < TOL;
        req!(full, true, "The centre came out at {}.", under);
        let anti = orthographic_cos_c(31.9535, -64.1429, -31.9535, 115.8571);
        let behind_us = (anti + 1.0).abs() < TOL;
        req!(behind_us, true, "The antipode came out at {}.", anti);
        // On the horizon it is zero, and the point lands exactly on the rim.
        let rim = orthographic_cos_c(0.0, 90.0, 0.0, 0.0);
        let level = rim.abs() < TOL;
        req!(level, true, "A quarter turn away came out at {}.", rim);
        let p = orthographic(0.0, 90.0, 0.0, 0.0, 1.0);
        let on = (p.x.hypot(p.y) - ORTHOGRAPHIC_HALF_WIDTH).abs() < TOL;
        req!(on, true, "The horizon came out {} from the centre.", p.x.hypot(p.y));
        // And nothing escapes the disc, near side or far.
        let mut lat = -90.0;
        while lat <= 90.0 {
            let mut lng = -180.0;
            while lng <= 180.0 {
                let p = orthographic(lat, lng, -31.9535, 115.8571, 1.0);
                let inside = p.x.hypot(p.y) <= ORTHOGRAPHIC_HALF_WIDTH + TOL;
                req!(inside, true, "{}, {} escaped the disc at {}, {}.", lat, lng, p.x, p.y);
                lng += 3.0;
            }
            lat += 3.0;
        }
        Ok(())
    }

    #[test]
    fn test_the_globe_turns_without_changing_shape_08() -> Outcome<()> {
        // Turning the globe by a degree of longitude and asking for a position a degree
        // further east is the same picture, because only the difference matters.
        let a = orthographic(12.0, 45.0, 0.0, 30.0, 1.0);
        let b = orthographic(12.0, 15.0, 0.0, 0.0, 1.0);
        let same_x = (a.x - b.x).abs() < TOL;
        let same_y = (a.y - b.y).abs() < TOL;
        req!(same_x, true, "Turning moved x to {} rather than {}.", a.x, b.x);
        req!(same_y, true, "Turning moved y to {} rather than {}.", a.y, b.y);
        // A radius scales the disc and nothing else.
        let one = orthographic(-33.8688, 151.2093, -31.9535, 115.8571, 1.0);
        let big = orthographic(-33.8688, 151.2093, -31.9535, 115.8571, 320.0);
        let scaled_x = (big.x - one.x * 320.0).abs() < 1.0e-9;
        let scaled_y = (big.y - one.y * 320.0).abs() < 1.0e-9;
        req!(scaled_x, true, "x did not scale by the radius.");
        req!(scaled_y, true, "y did not scale by the radius.");
        // A longitude past the antimeridian comes round rather than off the globe.
        let round = orthographic(0.0, 190.0, 0.0, 170.0, 1.0);
        let west = orthographic(0.0, -170.0, 0.0, 170.0, 1.0);
        let wrapped = (round.x - west.x).abs() < TOL;
        req!(wrapped, true, "190° east did not wrap to 170° west.");
        Ok(())
    }

    #[test]
    fn test_a_fix_just_outside_its_range_still_projects_05() -> Outcome<()> {
        let over = equal_earth_unit(90.000_000_1, 0.0);
        let finite = over.y.is_finite();
        req!(finite, true, "A latitude a hair over the pole gave {}.", over.y);
        let clamped = (over.y - EQUAL_EARTH_HALF_HEIGHT).abs() < TOL;
        req!(clamped, true, "It did not clamp to the pole.");
        let round = equal_earth_unit(0.0, 190.0);
        let west = equal_earth_unit(0.0, -170.0);
        let wrapped = (round.x - west.x).abs() < TOL;
        req!(wrapped, true, "190° east did not wrap to 170° west.");
        Ok(())
    }
}
