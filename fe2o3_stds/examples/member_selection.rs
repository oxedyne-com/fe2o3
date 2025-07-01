//! Example showing how to use generic person generation with member filters.

use oxedyne_fe2o3_core::prelude::*;
use oxedyne_fe2o3_stds::{
    person::{Person, PersonGenConfig, PersonFilter},
    person_filters::{MembershipFilter, LeadershipFilter, DevotionFilter},
};

fn main() -> Outcome<()> {
    // Configure generic person generation.
    let mut gen_config = PersonGenConfig::default();
    gen_config.current_year = 2024;
    gen_config.min_age = 18;
    gen_config.max_age = 85;
    
    // Create membership filter.
    let membership_filter = MembershipFilter {
        min_age: 25,
        max_age: 70,
        exception_probability: 0.1, // 10% chance of exception
        ..Default::default()
    };
    
    // Create leadership filter.
    let leadership_filter = LeadershipFilter::default();
    
    // Create devotion filter.
    let devotion_filter = DevotionFilter::default();
    
    println!("Generating 1000 random persons and applying filters...\n");
    
    let mut total_persons = 0;
    let mut members = Vec::new();
    let mut leaders = Vec::new();
    let mut high_devotion = Vec::new();
    
    // Generate persons until we have enough members.
    while members.len() < 100 {
        total_persons += 1;
        
        // Generate random global person.
        let person = res!(Person::generate_random(&gen_config));
        
        // Apply membership filter.
        if membership_filter.passes(&person) {
            let devotion_score = devotion_filter.devotion_score(&person);
            
            // Check for leadership potential.
            if leadership_filter.passes(&person) {
                leaders.push(person.clone());
            }
            
            // Check for high devotion.
            if devotion_score > 0.6 {
                high_devotion.push((person.clone(), devotion_score));
            }
            
            members.push(person);
        }
    }
    
    // Print statistics.
    println!("Total persons generated: {}", total_persons);
    println!("Members selected: {}", members.len());
    println!("Selection rate: {:.1}%", (members.len() as f64 / total_persons as f64) * 100.0);
    println!("\nAmong members:");
    println!("  Leaders: {} ({:.1}%)", leaders.len(), (leaders.len() as f64 / members.len() as f64) * 100.0);
    println!("  High devotion: {} ({:.1}%)", high_devotion.len(), (high_devotion.len() as f64 / members.len() as f64) * 100.0);
    
    // Show some examples.
    println!("\nExample members:");
    for (i, member) in members.iter().take(5).enumerate() {
        println!("  {}. {} (age {}, {})", 
            i + 1,
            member.full_name(),
            member.age,
            format!("{:?}", member.education)
        );
    }
    
    if !leaders.is_empty() {
        println!("\nExample leaders:");
        for (i, leader) in leaders.iter().take(3).enumerate() {
            println!("  {}. {} (age {}, {})", 
                i + 1,
                leader.full_name(),
                leader.age,
                format!("{:?}", leader.education)
            );
        }
    }
    
    Ok(())
}