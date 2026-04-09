//! Weather system affecting combat accuracy, sight range, and smoke behavior.
//!
//! Weather is rolled per-mission from the mission file's weather probability table.

use rand::Rng;
use serde::{Deserialize, Serialize};
use tracing::{debug, info};

use ow_data::mission::WeatherTable;

/// Active weather condition during a mission.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum Weather {
    Clear,
    Foggy,
    Overcast,
    LightRain,
    HeavyRain,
    Storm,
}

impl Weather {
    /// Accuracy multiplier applied to all ranged attacks.
    ///
    /// 1.0 = no penalty, lower = harder to hit.
    pub fn accuracy_modifier(&self) -> f32 {
        match self {
            Weather::Clear => 1.0,
            Weather::Foggy => 0.75,
            Weather::Overcast => 0.95,
            Weather::LightRain => 0.85,
            Weather::HeavyRain => 0.65,
            Weather::Storm => 0.45,
        }
    }

    /// Sight range multiplier.
    ///
    /// 1.0 = full sight, lower = reduced visibility.
    pub fn sight_range_modifier(&self) -> f32 {
        match self {
            Weather::Clear => 1.0,
            Weather::Foggy => 0.5,
            Weather::Overcast => 0.9,
            Weather::LightRain => 0.8,
            Weather::HeavyRain => 0.6,
            Weather::Storm => 0.4,
        }
    }

    /// Smoke grenade effectiveness modifier.
    ///
    /// > 1.0 = smoke lasts longer / spreads more, < 1.0 = disperses faster.
    pub fn smoke_modifier(&self) -> f32 {
        match self {
            Weather::Clear => 1.0,
            Weather::Foggy => 1.3,     // still air, smoke lingers
            Weather::Overcast => 1.1,
            Weather::LightRain => 0.9, // rain dampens smoke slightly
            Weather::HeavyRain => 0.6, // rain knocks smoke down
            Weather::Storm => 0.3,     // wind disperses smoke quickly
        }
    }
}

impl std::fmt::Display for Weather {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Weather::Clear => write!(f, "Clear"),
            Weather::Foggy => write!(f, "Foggy"),
            Weather::Overcast => write!(f, "Overcast"),
            Weather::LightRain => write!(f, "Light Rain"),
            Weather::HeavyRain => write!(f, "Heavy Rain"),
            Weather::Storm => write!(f, "Storm"),
        }
    }
}

/// Roll weather from a mission's probability table using weighted random selection.
///
/// The table contains percentage weights that should sum to 100.
/// Each weather type occupies a range proportional to its weight.
pub fn roll_weather(table: &WeatherTable) -> Weather {
    roll_weather_with_rng(table, &mut rand::thread_rng())
}

/// Roll weather with an explicit RNG (for testing / determinism).
pub fn roll_weather_with_rng<R: Rng>(table: &WeatherTable, rng: &mut R) -> Weather {
    let total = table.clear as u32
        + table.foggy as u32
        + table.overcast as u32
        + table.light_rain as u32
        + table.heavy_rain as u32
        + table.storm as u32;

    // Guard against an empty/zero table
    if total == 0 {
        info!("Weather table sums to 0, defaulting to Clear");
        return Weather::Clear;
    }

    let roll = rng.gen_range(0..total);

    let mut cursor = 0u32;
    let result = if roll < { cursor += table.clear as u32; cursor } {
        Weather::Clear
    } else if roll < { cursor += table.foggy as u32; cursor } {
        Weather::Foggy
    } else if roll < { cursor += table.overcast as u32; cursor } {
        Weather::Overcast
    } else if roll < { cursor += table.light_rain as u32; cursor } {
        Weather::LightRain
    } else if roll < { cursor += table.heavy_rain as u32; cursor } {
        Weather::HeavyRain
    } else {
        Weather::Storm
    };

    debug!(
        %result,
        roll,
        total,
        clear = table.clear,
        foggy = table.foggy,
        overcast = table.overcast,
        light_rain = table.light_rain,
        heavy_rain = table.heavy_rain,
        storm = table.storm,
        "Rolled weather"
    );

    result
}

#[cfg(test)]
mod tests {
    use super::*;
    use rand::rngs::mock::StepRng;

    fn table_clear_dominant() -> WeatherTable {
        WeatherTable {
            clear: 80,
            foggy: 5,
            overcast: 10,
            light_rain: 5,
            heavy_rain: 0,
            storm: 0,
        }
    }

    fn table_even() -> WeatherTable {
        WeatherTable {
            clear: 10,
            foggy: 10,
            overcast: 10,
            light_rain: 10,
            heavy_rain: 10,
            storm: 10,
        }
    }

    #[test]
    fn roll_zero_gives_first_bucket() {
        let table = table_even();
        // StepRng(0, 0) always returns 0 -> gen_range(0..60) = 0 -> Clear
        let mut rng = StepRng::new(0, 0);
        assert_eq!(roll_weather_with_rng(&table, &mut rng), Weather::Clear);
    }

    #[test]
    fn weather_modifiers_clear_baseline() {
        assert_eq!(Weather::Clear.accuracy_modifier(), 1.0);
        assert_eq!(Weather::Clear.sight_range_modifier(), 1.0);
        assert_eq!(Weather::Clear.smoke_modifier(), 1.0);
    }

    #[test]
    fn storm_worst_accuracy() {
        assert!(Weather::Storm.accuracy_modifier() < Weather::HeavyRain.accuracy_modifier());
        assert!(Weather::HeavyRain.accuracy_modifier() < Weather::Clear.accuracy_modifier());
    }

    #[test]
    fn fog_worst_visibility() {
        assert!(Weather::Foggy.sight_range_modifier() < Weather::Overcast.sight_range_modifier());
    }

    #[test]
    fn zero_table_defaults_to_clear() {
        let table = WeatherTable {
            clear: 0,
            foggy: 0,
            overcast: 0,
            light_rain: 0,
            heavy_rain: 0,
            storm: 0,
        };
        assert_eq!(roll_weather(&table), Weather::Clear);
    }

    #[test]
    fn clear_dominant_table_usually_clear() {
        let table = table_clear_dominant();
        let mut clear_count = 0;
        let mut rng = rand::thread_rng();
        for _ in 0..1000 {
            if roll_weather_with_rng(&table, &mut rng) == Weather::Clear {
                clear_count += 1;
            }
        }
        // With 80% weight, we should get clear at least 700 times out of 1000
        assert!(
            clear_count >= 700,
            "Expected ~800 clears, got {clear_count}"
        );
    }
}
