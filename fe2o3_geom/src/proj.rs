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
//! Only the Equal Earth projection is here so far. Further projections belong beside it in
//! this module rather than in a module of their own.

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
