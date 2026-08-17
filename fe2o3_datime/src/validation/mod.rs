//! Validation of date and time values against rules, profiles and business conditions.
//!
//! [Written with AI entirely](https://need2know.ai/entirely-ai/code)\
//! Anthropic Claude

pub mod rules;
pub mod validator;
pub mod analytics;
pub mod cache;
pub mod conditional;
pub mod parallel;
pub mod profile;

pub use self::{
    rules::{ValidationRule, ValidationRules},
    validator::{CalClockValidator, ValidationError, ValidationResult},
    analytics::{ValidationAnalytics, ValidationMetrics, ValidationReport},
    cache::{CachedValidator, ValidationCache},
    conditional::{ConditionalRule, ValidationCondition},
    parallel::{ParallelValidator, BatchValidationResult},
    profile::{ValidationProfile, ProfileBuilder, ProfileRegistry, StandardProfiles},
};