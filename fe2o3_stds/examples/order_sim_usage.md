# Using Generic Person Generation in order_sim

This example shows how to refactor your order_sim to use the generic person generation from fe2o3_stds.

## Before (Self-Selected Member Creation)

```rust
// In order_sim - tightly coupled member creation
impl Member {
    fn new_random() -> Self {
        // All person attributes mixed with member-specific logic
        let age = rand::thread_rng().gen_range(25..65);
        let education = if rand::random() { Education::High } else { Education::Medium };
        // ... more member-biased generation
    }
}
```

## After (Separated Concerns)

```rust
use oxedyne_fe2o3_stds::{
    person::{Person, PersonGenConfig, PersonFilter},
    person_filters::{MembershipFilter, DevotionFilter},
};

// In order_sim
pub struct Member {
    person: Person,
    joined_date: CalendarDate,
    devotion_level: f64,
    status: MemberStatus,
}

impl Member {
    /// Creates a member from a person who passed the membership filter.
    pub fn from_person(person: Person, devotion_filter: &DevotionFilter) -> Self {
        Self {
            devotion_level: devotion_filter.devotion_score(&person),
            person,
            joined_date: CalendarDate::today(),
            status: MemberStatus::Active,
        }
    }
}

// Simulation setup
pub fn create_organisation_members(target_count: usize) -> Outcome<Vec<Member>> {
    let gen_config = PersonGenConfig {
        current_year: 2024,
        min_age: 18,
        max_age: 90,
        // Configure general population parameters
        ..Default::default()
    };
    
    let membership_filter = MembershipFilter {
        min_age: 25,
        max_age: 70,
        min_education: EducationLevel::Secondary,
        require_middle_class_plus: true,
        exception_probability: 0.05, // 5% exceptions
        ..Default::default()
    };
    
    let devotion_filter = DevotionFilter::default();
    
    let mut members = Vec::new();
    
    // Generate global population and filter for members
    while members.len() < target_count {
        // Generate random person from global population
        let person = res!(Person::generate_random(&gen_config));
        
        // Apply membership selection filter
        if membership_filter.passes(&person) {
            let member = Member::from_person(person, &devotion_filter);
            members.push(member);
        }
    }
    
    Ok(members)
}
```

## Benefits

1. **Separation of Concerns**: Generic person generation is separate from membership logic.
2. **Reusability**: The Person struct and generation can be used in other simulations.
3. **Testability**: Filters can be tested independently.
4. **Flexibility**: Easy to adjust selection criteria without changing person generation.
5. **Realistic Modelling**: Shows the self-selection process explicitly.

## Custom Filters

You can create custom filters for your specific organisation:

```rust
pub struct OntheismMemberFilter {
    base_filter: MembershipFilter,
    // Additional organisation-specific criteria
    philosophical_interest_required: bool,
    min_openness_score: f64,
}

impl PersonFilter for OntheismMemberFilter {
    fn passes(&self, person: &Person) -> bool {
        // First check base membership criteria
        if !self.base_filter.passes(person) {
            return false;
        }
        
        // Then apply organisation-specific filters
        // (would need additional person attributes for this)
        true
    }
}
```

## Migration Steps

1. Add `oxedyne_fe2o3_stds` to your Cargo.toml dependencies.
2. Replace direct member generation with person generation + filtering.
3. Move member-specific attributes to a Member struct that contains a Person.
4. Create appropriate filters for your organisation's selection criteria.
5. Test that the demographic distribution matches your requirements.