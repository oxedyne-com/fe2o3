//! Named bundles of validation rules.
//!
//! A profile carries a set of rules, a strictness flag and some metadata, and
//! turns into a validator on demand. A registry holds several by name, and a
//! set of standard profiles covers the usual cases.
//!
//! [Written entirely with AI](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

use crate::{
    validation::{
        CalClockValidator, ValidationRule, ValidationRules, ConditionalRule,
    },
    constant::{DayOfWeek, MonthOfYear},
};

use oxedyne_fe2o3_core::prelude::*;

use std::collections::{HashMap, HashSet};

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
    name:           String,
    description:    Option<String>,
    version:        String,
    tags:           Vec<String>,
    rules:          Vec<ValidationRule>,
    metadata:       HashMap<String, String>,
    strict:         bool,                       // builds a strict validator
}

impl ValidationProfile {
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

    pub fn description<S: Into<String>>(mut self, description: S) -> Self {
        self.description = Some(description.into());
        self
    }

    pub fn version<S: Into<String>>(mut self, version: S) -> Self {
        self.version = version.into();
        self
    }

    pub fn tag<S: Into<String>>(mut self, tag: S) -> Self {
        self.tags.push(tag.into());
        self
    }

    pub fn tags<S: Into<String>>(mut self, tags: Vec<S>) -> Self {
        for tag in tags {
            self.tags.push(tag.into());
        }
        self
    }

    pub fn rule(mut self, rule: ValidationRule) -> Self {
        self.rules.push(rule);
        self
    }

    pub fn rules(mut self, mut rules: Vec<ValidationRule>) -> Self {
        self.rules.append(&mut rules);
        self
    }

    pub fn metadata<K: Into<String>, V: Into<String>>(mut self, key: K, value: V) -> Self {
        self.metadata.insert(key.into(), value.into());
        self
    }

    pub fn strict(mut self, strict: bool) -> Self {
        self.strict = strict;
        self
    }

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

    pub fn name(&self) -> &str {
        &self.name
    }

    pub fn get_description(&self) -> Option<&str> {
        self.description.as_deref()
    }

    pub fn get_version(&self) -> &str {
        &self.version
    }

    pub fn get_tags(&self) -> &[String] {
        &self.tags
    }

    pub fn get_rules(&self) -> &[ValidationRule] {
        &self.rules
    }

    pub fn get_metadata(&self) -> &HashMap<String, String> {
        &self.metadata
    }

    pub fn is_strict(&self) -> bool {
        self.strict
    }

    /// Rules and tags accumulate; where a metadata key appears in both,
    /// the other profile's value wins.
    pub fn merge(mut self, other: ValidationProfile) -> Self {
        self.rules.extend(other.rules);
        self.tags.extend(other.tags);
        self.metadata.extend(other.metadata);
        self
    }
}

#[derive(Debug)]
pub struct ProfileBuilder {
    profile: ValidationProfile,
}

impl ProfileBuilder {
    pub fn new<S: Into<String>>(name: S) -> Self {
        Self {
            profile: ValidationProfile::new(name),
        }
    }

    pub fn description<S: Into<String>>(mut self, description: S) -> Self {
        self.profile = self.profile.description(description);
        self
    }

    pub fn version<S: Into<String>>(mut self, version: S) -> Self {
        self.profile = self.profile.version(version);
        self
    }

    pub fn tag<S: Into<String>>(mut self, tag: S) -> Self {
        self.profile = self.profile.tag(tag);
        self
    }

    pub fn strict(mut self, strict: bool) -> Self {
        self.profile = self.profile.strict(strict);
        self
    }

    pub fn rule(mut self, rule: ValidationRule) -> Self {
        self.profile = self.profile.rule(rule);
        self
    }

    pub fn business_hours_only(mut self) -> Self {
        self.profile = self.profile.rule(ValidationRules::business_hours());
        self
    }

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

    pub fn weekends_only(mut self) -> Self {
        self.profile = self.profile.rule(ValidationRules::weekends_only());
        self
    }

    pub fn no_holidays(mut self) -> Self {
        self.profile = self.profile.rule(ValidationRules::no_holidays());
        self
    }

    pub fn future_dates_only(mut self) -> Self {
        self.profile = self.profile.rule(ValidationRules::not_too_old(0));
        self
    }

    pub fn past_dates_only(mut self) -> Self {
        self.profile = self.profile.rule(ValidationRules::not_too_future(0));
        self
    }

    pub fn whole_minutes_only(mut self) -> Self {
        self.profile = self.profile.rule(ValidationRules::whole_minutes_only());
        self
    }

    pub fn whole_seconds_only(mut self) -> Self {
        self.profile = self.profile.rule(ValidationRules::whole_seconds_only());
        self
    }

    pub fn months_only(mut self, months: Vec<MonthOfYear>) -> Self {
        let month_set: HashSet<MonthOfYear> = months.into_iter().collect();
        self.profile = self.profile.rule(ValidationRules::allowed_months(month_set));
        self
    }

    pub fn hour_range(mut self, min_hour: u8, max_hour: u8) -> Self {
        self.profile = self.profile.rule(ValidationRules::hour_range(min_hour, max_hour));
        self
    }

    pub fn year_range(mut self, min_year: i32, max_year: i32) -> Self {
        self.profile = self.profile.rule(ValidationRules::year_range(min_year, max_year));
        self
    }

    pub fn conditional_business_hours(mut self) -> Self {
        self.profile = self.profile.rule(ConditionalRule::business_hours_by_day().into_rule());
        self
    }

    pub fn seasonal_hours(mut self) -> Self {
        self.profile = self.profile.rule(ConditionalRule::seasonal_hours().into_rule());
        self
    }

    pub fn holiday_scheduling(mut self) -> Self {
        self.profile = self.profile.rule(ConditionalRule::holiday_scheduling().into_rule());
        self
    }

    pub fn build(self) -> ValidationProfile {
        self.profile
    }
}

#[derive(Debug, Default)]
pub struct ProfileRegistry {
    profiles: HashMap<String, ValidationProfile>,
}

impl ProfileRegistry {
    pub fn new() -> Self {
        Self {
            profiles: HashMap::new(),
        }
    }

    pub fn register(&mut self, profile: ValidationProfile) -> Outcome<()> {
        if self.profiles.contains_key(profile.name()) {
            return Err(err!("Profile '{}' already exists", profile.name(); Duplicate));
        }
        
        self.profiles.insert(profile.name().to_string(), profile);
        Ok(())
    }

    pub fn get(&self, name: &str) -> Option<&ValidationProfile> {
        self.profiles.get(name)
    }

    pub fn get_mut(&mut self, name: &str) -> Option<&mut ValidationProfile> {
        self.profiles.get_mut(name)
    }

    pub fn remove(&mut self, name: &str) -> Option<ValidationProfile> {
        self.profiles.remove(name)
    }

    pub fn list_names(&self) -> Vec<String> {
        self.profiles.keys().cloned().collect()
    }

    pub fn list_profiles(&self) -> Vec<&ValidationProfile> {
        self.profiles.values().collect()
    }

    pub fn find_by_tag(&self, tag: &str) -> Vec<&ValidationProfile> {
        self.profiles
            .values()
            .filter(|profile| profile.get_tags().contains(&tag.to_string()))
            .collect()
    }

    pub fn clear(&mut self) {
        self.profiles.clear();
    }

    pub fn len(&self) -> usize {
        self.profiles.len()
    }

    pub fn is_empty(&self) -> bool {
        self.profiles.is_empty()
    }
}

pub struct StandardProfiles;

impl StandardProfiles {
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