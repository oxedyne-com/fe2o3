//! Generic person generation and modelling.

use oxedyne_fe2o3_core::{
    prelude::*,
    rand::Rand,
};

/// Represents a generic person with basic attributes.
#[derive(Clone, Debug)]
pub struct Person {
    /// Given name.
    pub given_name: String,
    /// Family name.
    pub family_name: String,
    /// Age in years.
    pub age: u8,
    /// Birth year.
    pub birth_year: i32,
    /// Gender.
    pub gender: Gender,
    /// Cultural background.
    pub culture: String,
    /// Email address.
    pub email: Option<String>,
    /// Phone number.
    pub phone: Option<String>,
    /// Country of residence.
    pub country: String,
    /// City of residence.
    pub city: String,
    /// Occupation.
    pub occupation: Option<String>,
    /// Education level.
    pub education: EducationLevel,
    /// Income bracket.
    pub income_bracket: IncomeBracket,
}

/// Gender representation.
#[derive(Clone, Debug, PartialEq)]
pub enum Gender {
    Male,
    Female,
    Other,
}

/// Education level categories.
#[derive(Clone, Debug, PartialEq)]
pub enum EducationLevel {
    None,
    Primary,
    Secondary,
    Tertiary,
    Postgraduate,
}

/// Income bracket categories.
#[derive(Clone, Debug, PartialEq)]
pub enum IncomeBracket {
    Low,
    LowerMiddle,
    Middle,
    UpperMiddle,
    High,
}

/// Configuration for random person generation.
#[derive(Clone, Debug)]
pub struct PersonGenConfig {
    /// Current year for age calculations.
    pub current_year: i32,
    /// Minimum age.
    pub min_age: u8,
    /// Maximum age.
    pub max_age: u8,
    /// Available cultures to choose from.
    pub cultures: Vec<String>,
    /// Gender distribution weights [male, female, other].
    pub gender_weights: [f64; 3],
    /// Education level distribution weights.
    pub education_weights: Vec<f64>,
    /// Income bracket distribution weights.
    pub income_weights: Vec<f64>,
    /// Whether to generate contact details.
    pub generate_contact: bool,
    /// Whether to generate occupation.
    pub generate_occupation: bool,
}

impl Default for PersonGenConfig {
    fn default() -> Self {
        Self {
            current_year: 2024,
            min_age: 18,
            max_age: 80,
            cultures: vec!["Western".to_string()],
            gender_weights: [0.495, 0.495, 0.01], // Typical distribution
            education_weights: vec![0.05, 0.15, 0.35, 0.35, 0.10],
            income_weights: vec![0.20, 0.25, 0.30, 0.20, 0.05],
            generate_contact: true,
            generate_occupation: true,
        }
    }
}

impl Person {
    /// Creates a new person with specified attributes.
    pub fn new(
        given_name: String,
        family_name: String,
        age: u8,
        birth_year: i32,
        gender: Gender,
        culture: String,
    ) -> Self {
        Self {
            given_name,
            family_name,
            age,
            birth_year,
            gender,
            culture,
            email: None,
            phone: None,
            country: String::new(),
            city: String::new(),
            occupation: None,
            education: EducationLevel::Secondary,
            income_bracket: IncomeBracket::Middle,
        }
    }

    /// Generates a random person based on configuration.
    pub fn generate_random(config: &PersonGenConfig) -> Outcome<Self> {
        
        // Select gender based on weights.
        let gender = res!(select_weighted(
            &[Gender::Male, Gender::Female, Gender::Other],
            &config.gender_weights
        ));
        
        // Generate age.
        let age = Rand::in_range(config.min_age, config.max_age);
        let birth_year = config.current_year - age as i32;
        
        // Select culture.
        let culture = if config.cultures.is_empty() {
            "Western".to_string()
        } else {
            let idx = Rand::in_range(0, config.cultures.len() - 1);
            config.cultures[idx].clone()
        };
        
        // Generate names based on culture and gender.
        let (given_name, family_name) = res!(generate_names(&culture, &gender));
        
        // Create base person.
        let mut person = Self::new(
            given_name,
            family_name,
            age,
            birth_year,
            gender,
            culture,
        );
        
        // Set education level.
        person.education = res!(select_weighted(
            &[
                EducationLevel::None,
                EducationLevel::Primary,
                EducationLevel::Secondary,
                EducationLevel::Tertiary,
                EducationLevel::Postgraduate,
            ],
            &config.education_weights
        ));
        
        // Set income bracket.
        person.income_bracket = res!(select_weighted(
            &[
                IncomeBracket::Low,
                IncomeBracket::LowerMiddle,
                IncomeBracket::Middle,
                IncomeBracket::UpperMiddle,
                IncomeBracket::High,
            ],
            &config.income_weights
        ));
        
        // Generate location.
        let (country, city) = res!(generate_location(&person.culture));
        person.country = country;
        person.city = city;
        
        // Generate contact details if requested.
        if config.generate_contact {
            person.email = Some(res!(generate_email(&person)));
            person.phone = Some(res!(generate_phone(&person.country)));
        }
        
        // Generate occupation if requested.
        if config.generate_occupation {
            person.occupation = Some(res!(generate_occupation(&person.education, &person.income_bracket)));
        }
        
        Ok(person)
    }
    
    /// Full name combining given and family names.
    pub fn full_name(&self) -> String {
        format!("{} {}", self.given_name, self.family_name)
    }
}

/// Filter trait for selecting persons based on criteria.
pub trait PersonFilter {
    /// Tests if a person passes the filter.
    fn passes(&self, person: &Person) -> bool;
}

/// Composite filter combining multiple filters.
pub struct CompositeFilter {
    filters: Vec<Box<dyn PersonFilter>>,
}

impl CompositeFilter {
    /// Creates a new composite filter.
    pub fn new() -> Self {
        Self {
            filters: Vec::new(),
        }
    }
    
    /// Adds a filter to the composite.
    pub fn add_filter(mut self, filter: Box<dyn PersonFilter>) -> Self {
        self.filters.push(filter);
        self
    }
}

impl PersonFilter for CompositeFilter {
    fn passes(&self, person: &Person) -> bool {
        self.filters.iter().all(|f| f.passes(person))
    }
}

/// Age range filter.
pub struct AgeRangeFilter {
    min: u8,
    max: u8,
}

impl AgeRangeFilter {
    pub fn new(min: u8, max: u8) -> Self {
        Self { min, max }
    }
}

impl PersonFilter for AgeRangeFilter {
    fn passes(&self, person: &Person) -> bool {
        person.age >= self.min && person.age <= self.max
    }
}

/// Helper function to select from weighted options.
fn select_weighted<T: Clone>(
    options: &[T],
    weights: &[f64],
) -> Outcome<T> {
    if options.len() != weights.len() {
        return Err(err!(
            "Options and weights must have same length"; 
            Invalid, Input
        ));
    }
    
    let total: f64 = weights.iter().sum();
    let mut choice = Rand::value::<f64>() * total;
    
    for (i, weight) in weights.iter().enumerate() {
        choice -= weight;
        if choice <= 0.0 {
            return Ok(options[i].clone());
        }
    }
    
    // Fallback to last option.
    Ok(options[options.len() - 1].clone())
}

/// Generates culturally appropriate names.
fn generate_names(_culture: &str, _gender: &Gender) -> Outcome<(String, String)> {
    // Placeholder implementation.
    // In real implementation, would use culture-specific name lists.
    Ok((
        format!("Given{}", Rand::value::<u16>()),
        format!("Family{}", Rand::value::<u16>()),
    ))
}

/// Generates location based on culture.
fn generate_location(_culture: &str) -> Outcome<(String, String)> {
    // Placeholder implementation.
    // In real implementation, would use culture-specific location data.
    Ok(("Country".to_string(), "City".to_string()))
}

/// Generates email address.
fn generate_email(person: &Person) -> Outcome<String> {
    Ok(format!(
        "{}.{}@example.com",
        person.given_name.to_lowercase(),
        person.family_name.to_lowercase()
    ))
}

/// Generates phone number based on country.
fn generate_phone(_country: &str) -> Outcome<String> {
    // Placeholder implementation.
    Ok(format!("+1555{:07}", Rand::value::<u32>() % 10000000))
}

/// Generates occupation based on education and income.
fn generate_occupation(_education: &EducationLevel, _income: &IncomeBracket) -> Outcome<String> {
    // Placeholder implementation.
    Ok("Professional".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_person_generation() {
        let config = PersonGenConfig::default();
        let person = Person::generate_random(&config).unwrap();
        
        assert!(person.age >= config.min_age);
        assert!(person.age <= config.max_age);
        assert_eq!(person.birth_year, config.current_year - person.age as i32);
    }
    
    #[test]
    fn test_age_filter() {
        let filter = AgeRangeFilter::new(25, 35);
        
        let person1 = Person::new(
            "John".to_string(),
            "Doe".to_string(),
            30,
            1994,
            Gender::Male,
            "Western".to_string(),
        );
        assert!(filter.passes(&person1));
        
        let person2 = Person::new(
            "Jane".to_string(),
            "Smith".to_string(),
            20,
            2004,
            Gender::Female,
            "Western".to_string(),
        );
        assert!(!filter.passes(&person2));
    }
}