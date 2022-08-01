use std::io;
use std::io::Write;

use anyhow::{Context, Result};
use brotli2::write::BrotliEncoder;

use chrono::{Date, DateTime, Duration, NaiveDate, Utc};
use encoding::label::encoding_from_whatwg_label;
use encoding::DecoderTrap;

use crate::Mode;

const HOUR_ANGLE_TO_MINUTES_FACTOR: f64 = 4.0;
pub fn compress_body(/*new_headers: &mut HeaderMap, */ body_bytes: &Vec<u8>,) -> Result<Vec<u8>> {
    let buf = Vec::new();
    let mut compressor = BrotliEncoder::new(buf, 11);
    let _ = compressor
        .write(body_bytes.as_slice())
        .context("compress error")?;
    let result = compressor.finish()?;
    Ok(result)
}

pub fn to_utf8(orig: &[u8], charset: &str) -> Result<Vec<u8>> {
    let e1 = encoding_from_whatwg_label(charset).context("encoding error")?;
    let s = e1.decode(orig, DecoderTrap::Strict).map_err(|_| {
        io::Error::new(
            io::ErrorKind::InvalidData,
            format!("decode error: {}", charset),
        )
    })?;
    Ok(s.into_bytes())
}

/// Calculates the approximate sunset and sunrise times at a given latitude, longitude, and altitude
///
/// # Arguments
///
/// * `date` - The date on which to calculate the sunset and sunrise
/// * `latitude` - The latitude at which to calculate the times. Expressed as degrees
/// * `longitude` - The longitude at which to calculate the times. Expressed as degrees
/// * `elevation` - The elevation at which to calculate the times. Expressed as meters above sea level
///
/// # Return value
///
/// Returns a tuple of `(sunrise,sunset)`
///
/// # Examples
///
/// ```
/// //Calculate the sunset and sunrise times today at Sheffield university's new computer science building
/// let times = sun_times(Utc::today(),53.38,-1.48,100.0);
/// println!("Sunrise: {}, Sunset: {}",times.0,times.1);
/// ```
fn sun_times(
    date: Date<Utc>,
    latitude: f64,
    longitude: f64,
    elevation: f64,
) -> (DateTime<Utc>, DateTime<Utc>) {
    //see https://en.wikipedia.org/wiki/Sunrise_equation

    const ARGUMENT_OF_PERIHELION: f64 = 102.9372;

    let elevation_correction = -2.076 * (elevation.sqrt()) / 60.0;

    let jan_2000 = Date::<Utc>::from_utc(NaiveDate::from_ymd(2000, 1, 1), Utc);
    let time_since_2000: Duration = date.signed_duration_since(jan_2000);

    let mean_solar_time = (time_since_2000.num_days() as f64) + 0.0008 - (longitude / 360.0);
    let solar_mean_anomaly = (357.5291 + 0.98560028 * mean_solar_time) % 360.0;
    let center = 1.9148 * solar_mean_anomaly.to_radians().sin()
        + 0.0200 * (2.0 * solar_mean_anomaly).to_radians().sin()
        + 0.0003 * (3.0 * solar_mean_anomaly).to_radians().sin();
    let ecliptic_longitude = (solar_mean_anomaly + center + 180.0 + ARGUMENT_OF_PERIHELION) % 360.0;

    let declination =
        (ecliptic_longitude.to_radians().sin() * (23.44f64).to_radians().sin()).asin();
    let hour_angle = (((-0.83 + elevation_correction).to_radians().sin()
        - (latitude.to_radians().sin() * declination.sin()))
        / (latitude.to_radians().cos() * declination.cos()))
    .acos()
    .to_degrees();

    let solar_transit = mean_solar_time + 0.0053 * solar_mean_anomaly.to_radians().sin()
        - 0.0069 * (2.0 * ecliptic_longitude).to_radians().sin();
    let solar_transit_date = jan_2000 + Duration::days(solar_transit.round() as i64);
    let solar_transit_date = solar_transit_date.and_hms(12, 0, 0)
        + Duration::seconds(
            ((solar_transit * 24.0 * 60.0 * 60.0) % (24.0 * 60.0 * 60.0)).round() as i64,
        );

    let minutes = Duration::minutes((hour_angle * HOUR_ANGLE_TO_MINUTES_FACTOR).round() as i64);
    let set = solar_transit_date + minutes;
    let rise = solar_transit_date - minutes;
    (rise, set)
}

pub fn get_mode(lat: f64, long: f64, elev: f64) -> Mode {
    let times = sun_times(Utc::today(), lat, long, elev);
    let now = Utc::now().time();
    if times.0.time().cmp(&now).is_le() && times.1.time().cmp(&now).is_ge() {
        return Mode::DAY;
    }
    Mode::NIGHT
}
