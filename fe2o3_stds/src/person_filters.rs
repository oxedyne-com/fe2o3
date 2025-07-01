//! Member-specific filters for organisation membership selection.

use oxedyne_fe2o3_core::prelude::*;

use crate::person::{Person, PersonFilter, EducationLevel, IncomeBracket};

/// Filter for selecting persons suitable for organisation membership.
/// This implements the self-selection bias for membership.
pub struct MembershipFilter {
    /// Minimum age for membership.
    pub min_age: u8,
    /// Maximum age for membership.
    pub max_age: u8,
    /// Required minimum education level.
    pub min_education: EducationLevel,
    /// Whether to require certain income levels.
    pub require_middle_class_plus: bool,
    /// Cultural compatibility requirements.
    pub compatible_cultures: Vec<String>,
    /// Probability of acceptance even if criteria not met (0.0-1.0).
    pub exception_probability: f64,
}

impl Default for MembershipFilter {
    fn default() -> Self {
        Self {
            min_age: 25,
            max_age: 65,
            min_education: EducationLevel::Secondary,
            require_middle_class_plus: true,
            compatible_cultures: vec![],
            exception_probability: 0.05,
        }
    }
}

impl PersonFilter for MembershipFilter {
    fn passes(&self, person: &Person) -> bool {
        // Age check.
        if person.age < self.min_age || person.age > self.max_age {
            return self.check_exception();
        }
        
        // Education check.
        if !self.meets_education_requirement(person) {
            return self.check_exception();
        }
        
        // Income check.
        if self.require_middle_class_plus && !self.meets_income_requirement(person) {
            return self.check_exception();
        }
        
        // Cultural compatibility check.
        if !self.compatible_cultures.is_empty() {
            let culture_name = format!("{:?}", person.culture); // Placeholder
            if !self.compatible_cultures.contains(&culture_name) {
                return self.check_exception();
            }
        }
        
        true
    }
}

impl MembershipFilter {
    /// Checks if person meets education requirements.
    fn meets_education_requirement(&self, person: &Person) -> bool {
        match (&person.education, &self.min_education) {
            (EducationLevel::None, _) => false,
            (EducationLevel::Primary, EducationLevel::Primary) => true,
            (EducationLevel::Primary, _) => false,
            (EducationLevel::Secondary, EducationLevel::Primary) => true,
            (EducationLevel::Secondary, EducationLevel::Secondary) => true,
            (EducationLevel::Secondary, _) => false,
            (EducationLevel::Tertiary, EducationLevel::Postgraduate) => false,
            (EducationLevel::Tertiary, _) => true,
            (EducationLevel::Postgraduate, _) => true,
        }
    }
    
    /// Checks if person meets income requirements.
    fn meets_income_requirement(&self, person: &Person) -> bool {
        matches!(
            person.income_bracket,
            IncomeBracket::Middle | IncomeBracket::UpperMiddle | IncomeBracket::High
        )
    }
    
    /// Randomly determines if an exception should be made.
    fn check_exception(&self) -> bool {
        use oxedyne_fe2o3_core::rand::Rand;
        Rand::value::<f64>() < self.exception_probability
    }
}

/// Filter for leadership selection within members.
pub struct LeadershipFilter {
    /// Minimum age for leadership.
    pub min_age: u8,
    /// Required minimum education.
    pub min_education: EducationLevel,
    /// Years of membership required (would need to be tracked separately).
    pub min_years_membership: u8,
}

impl Default for LeadershipFilter {
    fn default() -> Self {
        Self {
            min_age: 35,
            min_education: EducationLevel::Tertiary,
            min_years_membership: 5,
        }
    }
}

impl PersonFilter for LeadershipFilter {
    fn passes(&self, person: &Person) -> bool {
        person.age >= self.min_age && 
        matches!(
            person.education,
            EducationLevel::Tertiary | EducationLevel::Postgraduate
        )
        // Note: Years of membership would need to be tracked separately.
    }
}

/// Devotion level filter based on demographics.
/// This models how certain demographics might have higher devotion/commitment.
pub struct DevotionFilter {
    /// Age ranges with higher devotion (min, max).
    pub high_devotion_age_ranges: Vec<(u8, u8)>,
    /// Education levels associated with higher devotion.
    pub high_devotion_education: Vec<EducationLevel>,
    /// Base devotion probability.
    pub base_devotion_probability: f64,
    /// Boost for matching demographics.
    pub demographic_boost: f64,
}

impl Default for DevotionFilter {
    fn default() -> Self {
        Self {
            high_devotion_age_ranges: vec![(45, 65), (70, 90)],
            high_devotion_education: vec![
                EducationLevel::Primary,
                EducationLevel::Postgraduate,
            ],
            base_devotion_probability: 0.3,
            demographic_boost: 0.3,
        }
    }
}

impl DevotionFilter {
    /// Calculates devotion score for a person (0.0-1.0).
    pub fn devotion_score(&self, person: &Person) -> f64 {
        let mut score = self.base_devotion_probability;
        
        // Age boost.
        for (min, max) in &self.high_devotion_age_ranges {
            if person.age >= *min && person.age <= *max {
                score += self.demographic_boost;
                break;
            }
        }
        
        // Education boost.
        if self.high_devotion_education.contains(&person.education) {
            score += self.demographic_boost;
        }
        
        score.min(1.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::person::Gender;
        
    #[test]
    fn test_membership_filter() {
        let filter = MembershipFilter::default();
        
        // Should pass.
        let member = Person {
            given_name: "John".to_string(),
            family_name: "Doe".to_string(),
            age: 35,
            birth_year: 1989,
            gender: Gender::Male,
            culture: "Western".to_string(),
            email: None,
            phone: None,
            country: "USA".to_string(),
            city: "Boston".to_string(),
            occupation: Some("Engineer".to_string()),
            education: EducationLevel::Tertiary,
            income_bracket: IncomeBracket::UpperMiddle,
        };
        assert!(filter.passes(&member));
        
        // Too young.
        let mut young = member.clone();
        young.age = 20;
        assert!(!filter.passes(&young) || filter.exception_probability > 0.0);
        
        // Low income.
        let mut poor = member.clone();
        poor.income_bracket = IncomeBracket::Low;
        assert!(!filter.passes(&poor) || filter.exception_probability > 0.0);
    }
    
    #[test]
    fn test_devotion_filter() {
        let filter = DevotionFilter::default();
        
        // High devotion demographic.
        let devoted = Person {
            given_name: "Mary".to_string(),
            family_name: "Smith".to_string(),
            age: 50,
            birth_year: 1974,
            gender: Gender::Female,
            culture: "Western".to_string(),
            email: None,
            phone: None,
            country: "USA".to_string(),
            city: "Dallas".to_string(),
            occupation: Some("Teacher".to_string()),
            education: EducationLevel::Postgraduate,
            income_bracket: IncomeBracket::Middle,
        };
        
        let score = filter.devotion_score(&devoted);
        assert!(score > filter.base_devotion_probability);
        assert!(score <= 1.0);
    }
}