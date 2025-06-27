use crate::{
    validation::{
        CalClockValidator, ValidationRule, ValidationRules, ConditionalRule,
    },
    constant::{DayOfWeek, MonthOfYear},
};

use oxedyne_fe2o3_core::prelude::*;

use std::collections::{HashMap, HashSet};

/// Comprehensive validation profile system for complex validation scenarios.
///
/// ValidationProfile provides a way to define, store, and apply complex
/// validation configurations for different use cases, environments, or
/// business contexts. Profiles can be serialised, shared, and dynamically
/// loaded to support flexible validation requirements.
///
/// # Examples
///
/// ```ignore
/// use oxedyne_fe2o3_datime::validation::{ValidationProfile, ProfileBuilder};
///
/// // Create a business scheduling profile
/// let profile = ProfileBuilder::new("business_scheduling")
///     .description("Standard business scheduling rules")
///     .business_hours_only()
///     .no_weekends()
///     .future_dates_only()
///     .build();
///
/// let validator = profile.create_validator();
/// ```
#[derive(Debug)]
pub struct ValidationProfile {
    /// Profile name.
    name: String,
    /// Profile description.
    description: Option<String>,
    /// Version of this profile.
    version: String,
    /// Tags for categorization.
    tags: Vec<String>,
    /// The validation rules in this profile.
    rules: Vec<ValidationRule>,
    /// Profile metadata.
    metadata: HashMap<String, String>,
    /// Whether this profile is strict (all rules must pass).
    strict: bool,
}

impl ValidationProfile {
    /// Creates a new validation profile.
    pub fn new<S: Into<String>>(name: S) -> Self {
        Self {
            name: name.into(),
            description: None,
            version: "1.0.0".to_string(),
            tags: Vec::new(),
            rules: Vec::new(),
            metadata: HashMap::new(),
            strict: true,
        }
    }

    /// Sets the profile description.
    pub fn description<S: Into<String>>(mut self, description: S) -> Self {
        self.description = Some(description.into());
        self
    }

    /// Sets the profile version.
    pub fn version<S: Into<String>>(mut self, version: S) -> Self {
        self.version = version.into();
        self
    }

    /// Adds a tag to this profile.
    pub fn tag<S: Into<String>>(mut self, tag: S) -> Self {
        self.tags.push(tag.into());
        self
    }

    /// Adds multiple tags to this profile.
    pub fn tags<S: Into<String>>(mut self, tags: Vec<S>) -> Self {
        for tag in tags {
            self.tags.push(tag.into());
        }
        self
    }

    /// Adds a validation rule to this profile.
    pub fn rule(mut self, rule: ValidationRule) -> Self {
        self.rules.push(rule);
        self
    }

    /// Adds multiple validation rules to this profile.
    pub fn rules(mut self, mut rules: Vec<ValidationRule>) -> Self {
        self.rules.append(&mut rules);
        self
    }

    /// Adds metadata to this profile.
    pub fn metadata<K: Into<String>, V: Into<String>>(mut self, key: K, value: V) -> Self {
        self.metadata.insert(key.into(), value.into());
        self
    }

    /// Sets whether this profile is strict.
    pub fn strict(mut self, strict: bool) -> Self {
        self.strict = strict;
        self
    }

    /// Creates a CalClockValidator from this profile.
    pub fn create_validator(self) -> CalClockValidator {
        let mut validator = if self.strict {
            CalClockValidator::strict()
        } else {
            CalClockValidator::new()
        };

        // Move rules into the validator since ValidationRule doesn't implement Clone
        for rule in self.rules {
            validator.add_rule(rule);
        }

        validator
    }

    /// Gets the profile name.
    pub fn name(&self) -> &str {
        &self.name
    }

    /// Gets the profile description.
    pub fn get_description(&self) -> Option<&str> {
        self.description.as_deref()
    }

    /// Gets the profile version.
    pub fn get_version(&self) -> &str {
        &self.version
    }

    /// Gets the profile tags.
    pub fn get_tags(&self) -> &[String] {
        &self.tags
    }

    /// Gets the validation rules.
    pub fn get_rules(&self) -> &[ValidationRule] {
        &self.rules
    }

    /// Gets profile metadata.
    pub fn get_metadata(&self) -> &HashMap<String, String> {
        &self.metadata
    }

    /// Checks if this profile is strict.
    pub fn is_strict(&self) -> bool {
        self.strict
    }

    /// Merges another profile into this one.
    pub fn merge(mut self, other: ValidationProfile) -> Self {
        self.rules.extend(other.rules);
        self.tags.extend(other.tags);
        self.metadata.extend(other.metadata);
        self
    }
}

/// Builder for creating validation profiles with common patterns.
#[derive(Debug)]
pub struct ProfileBuilder {
    profile: ValidationProfile,
}

impl ProfileBuilder {
    /// Creates a new profile builder.
    pub fn new<S: Into<String>>(name: S) -> Self {
        Self {
            profile: ValidationProfile::new(name),
        }
    }

    /// Sets the profile description.
    pub fn description<S: Into<String>>(mut self, description: S) -> Self {
        self.profile = self.profile.description(description);
        self
    }

    /// Sets the profile version.
    pub fn version<S: Into<String>>(mut self, version: S) -> Self {
        self.profile = self.profile.version(version);
        self
    }

    /// Adds a tag.
    pub fn tag<S: Into<String>>(mut self, tag: S) -> Self {
        self.profile = self.profile.tag(tag);
        self
    }

    /// Sets strict mode.
    pub fn strict(mut self, strict: bool) -> Self {
        self.profile = self.profile.strict(strict);
        self
    }

    /// Adds a custom validation rule.
    pub fn rule(mut self, rule: ValidationRule) -> Self {
        self.profile = self.profile.rule(rule);
        self
    }

    /// Adds business hours validation.
    pub fn business_hours_only(mut self) -> Self {
        self.profile = self.profile.rule(ValidationRules::business_hours());
        self
    }

    /// Excludes weekends.
    pub fn no_weekends(mut self) -> Self {
        let mut weekdays = HashSet::new();
        weekdays.insert(DayOfWeek::Monday);
        weekdays.insert(DayOfWeek::Tuesday);
        weekdays.insert(DayOfWeek::Wednesday);
        weekdays.insert(DayOfWeek::Thursday);
        weekdays.insert(DayOfWeek::Friday);
        
        self.profile = self.profile.rule(ValidationRules::allowed_weekdays(weekdays));
        self
    }

    /// Only allows weekends.
    pub fn weekends_only(mut self) -> Self {
        self.profile = self.profile.rule(ValidationRules::weekends_only());
        self
    }

    /// Excludes holidays.
    pub fn no_holidays(mut self) -> Self {
        self.profile = self.profile.rule(ValidationRules::no_holidays());
        self
    }

    /// Only allows future dates.
    pub fn future_dates_only(mut self) -> Self {
        self.profile = self.profile.rule(ValidationRules::not_too_old(0));
        self
    }

    /// Only allows past dates.
    pub fn past_dates_only(mut self) -> Self {
        self.profile = self.profile.rule(ValidationRules::not_too_future(0));
        self
    }

    /// Only allows whole minutes (no seconds/nanoseconds).
    pub fn whole_minutes_only(mut self) -> Self {
        self.profile = self.profile.rule(ValidationRules::whole_minutes_only());
        self
    }

    /// Only allows whole seconds (no nanoseconds).
    pub fn whole_seconds_only(mut self) -> Self {
        self.profile = self.profile.rule(ValidationRules::whole_seconds_only());
        self
    }

    /// Restricts to specific months.
    pub fn months_only(mut self, months: Vec<MonthOfYear>) -> Self {
        let month_set: HashSet<MonthOfYear> = months.into_iter().collect();
        self.profile = self.profile.rule(ValidationRules::allowed_months(month_set));
        self
    }

    /// Restricts to specific hour range.
    pub fn hour_range(mut self, min_hour: u8, max_hour: u8) -> Self {
        self.profile = self.profile.rule(ValidationRules::hour_range(min_hour, max_hour));
        self
    }

    /// Restricts to specific year range.
    pub fn year_range(mut self, min_year: i32, max_year: i32) -> Self {
        self.profile = self.profile.rule(ValidationRules::year_range(min_year, max_year));
        self
    }

    /// Adds conditional business hours (different for weekdays/weekends).
    pub fn conditional_business_hours(mut self) -> Self {
        self.profile = self.profile.rule(ConditionalRule::business_hours_by_day().into_rule());
        self
    }

    /// Adds seasonal hour restrictions.
    pub fn seasonal_hours(mut self) -> Self {
        self.profile = self.profile.rule(ConditionalRule::seasonal_hours().into_rule());
        self
    }

    /// Adds holiday scheduling rules.
    pub fn holiday_scheduling(mut self) -> Self {
        self.profile = self.profile.rule(ConditionalRule::holiday_scheduling().into_rule());
        self
    }

    /// Builds the final validation profile.
    pub fn build(self) -> ValidationProfile {
        self.profile
    }
}

/// Registry for managing multiple validation profiles.
#[derive(Debug, Default)]
pub struct ProfileRegistry {
    /// Stored profiles by name.
    profiles: HashMap<String, ValidationProfile>,
}

impl ProfileRegistry {
    /// Creates a new profile registry.
    pub fn new() -> Self {
        Self {
            profiles: HashMap::new(),
        }
    }

    /// Registers a validation profile.
    pub fn register(&mut self, profile: ValidationProfile) -> Outcome<()> {
        if self.profiles.contains_key(profile.name()) {
            return Err(err!("Profile '{}' already exists", profile.name(); Duplicate));
        }
        
        self.profiles.insert(profile.name().to_string(), profile);
        Ok(())
    }

    /// Gets a profile by name.
    pub fn get(&self, name: &str) -> Option<&ValidationProfile> {
        self.profiles.get(name)
    }

    /// Gets a mutable reference to a profile by name.
    pub fn get_mut(&mut self, name: &str) -> Option<&mut ValidationProfile> {
        self.profiles.get_mut(name)
    }

    /// Removes a profile by name.
    pub fn remove(&mut self, name: &str) -> Option<ValidationProfile> {
        self.profiles.remove(name)
    }

    /// Lists all profile names.
    pub fn list_names(&self) -> Vec<String> {
        self.profiles.keys().cloned().collect()
    }

    /// Lists all profiles.
    pub fn list_profiles(&self) -> Vec<&ValidationProfile> {
        self.profiles.values().collect()
    }

    /// Finds profiles by tag.
    pub fn find_by_tag(&self, tag: &str) -> Vec<&ValidationProfile> {
        self.profiles
            .values()
            .filter(|profile| profile.get_tags().contains(&tag.to_string()))
            .collect()
    }

    /// Clears all profiles.
    pub fn clear(&mut self) {
        self.profiles.clear();
    }

    /// Gets the number of registered profiles.
    pub fn len(&self) -> usize {
        self.profiles.len()
    }

    /// Checks if the registry is empty.
    pub fn is_empty(&self) -> bool {
        self.profiles.is_empty()
    }
}

/// Pre-built validation profiles for common use cases.
pub struct StandardProfiles;

impl StandardProfiles {
    /// Business scheduling profile (weekdays, business hours, no holidays).
    pub fn business_scheduling() -> ValidationProfile {
        ProfileBuilder::new("business_scheduling")
            .description("Standard business scheduling validation")
            .tag("business")
            .tag("scheduling")
            .version("1.0.0")
            .business_hours_only()
            .no_weekends()
            .no_holidays()
            .whole_minutes_only()
            .build()
    }

    /// Appointment booking profile with extended flexibility.
    pub fn appointment_booking() -> ValidationProfile {
        ProfileBuilder::new("appointment_booking")
            .description("Flexible appointment booking validation")
            .tag("appointment")
            .tag("booking")
            .version("1.0.0")
            .conditional_business_hours()
            .future_dates_only()
            .whole_minutes_only()
            .build()
    }

    /// Historical data entry profile (past dates only, strict validation).
    pub fn historical_data() -> ValidationProfile {
        ProfileBuilder::new("historical_data")
            .description("Historical data entry validation")
            .tag("historical")
            .tag("data")
            .version("1.0.0")
            .strict(true)
            .past_dates_only()
            .year_range(1900, 2024)
            .build()
    }

    /// Event scheduling profile with seasonal considerations.
    pub fn event_scheduling() -> ValidationProfile {
        ProfileBuilder::new("event_scheduling")
            .description("Event scheduling with seasonal hours")
            .tag("event")
            .tag("scheduling")
            .version("1.0.0")
            .seasonal_hours()
            .holiday_scheduling()
            .whole_minutes_only()
            .build()
    }

    /// Maintenance window profile (very restrictive).
    pub fn maintenance_window() -> ValidationProfile {
        ProfileBuilder::new("maintenance_window")
            .description("System maintenance window scheduling")
            .tag("maintenance")
            .tag("system")
            .version("1.0.0")
            .strict(true)
            .hour_range(2, 6) // 2 AM - 6 AM
            .weekends_only()
            .future_dates_only()
            .build()
    }

    /// Relaxed profile for testing and development.
    pub fn testing() -> ValidationProfile {
        ProfileBuilder::new("testing")
            .description("Relaxed validation for testing purposes")
            .tag("testing")
            .tag("development")
            .version("1.0.0")
            .strict(false)
            .year_range(1970, 2100)
            .build()
    }

    /// Creates a registry with all standard profiles.
    pub fn create_registry() -> ProfileRegistry {
        let mut registry = ProfileRegistry::new();
        
        let _ = registry.register(Self::business_scheduling());
        let _ = registry.register(Self::appointment_booking());
        let _ = registry.register(Self::historical_data());
        let _ = registry.register(Self::event_scheduling());
        let _ = registry.register(Self::maintenance_window());
        let _ = registry.register(Self::testing());
        
        registry
    }
}