use oxedize_fe2o3_core::new_enum;
use strum::Display;

#[derive(Clone, Copy, Debug, Display, Eq, PartialEq)]
#[repr(usize)]
pub enum Gender {
    Male,
    Female,
    NonBinary,
    Other,
    PreferNotToSay,
}

new_enum!(Gender;
    Male,
    Female,
    NonBinary,
    Other,
    PreferNotToSay,
);

impl Gender {
    /// Returns a random Gender variant with specified minority probability.
    /// 
    /// # Arguments
    /// * `p` - Probability (0.0 to 1.0) of selecting a minority gender (NonBinary, Other, PreferNotToSay).
    ///         The probability of Male or Female is (1-p), split equally between them.
    /// 
    /// # Example
    /// ```
    /// use oxedize_fe2o3_stds::culture::Gender;
    /// 
    /// // 10% chance of minority gender, 90% chance of Male/Female.
    /// let gender = Gender::rand_minority(0.1);
    /// 
    /// // 50% chance of minority gender, 50% chance of Male/Female.
    /// let gender = Gender::rand_minority(0.5);
    /// ```
    pub fn rand_minority(p: f64) -> Self {
        use ::oxedize_fe2o3_core::rand::Rand;
        
        let random_val: f64 = Rand::value();
        
        if random_val < (1.0 - p) {
            // Choose Male or Female with equal probability.
            if Rand::value::<f64>() < 0.5 {
                Self::Male
            } else {
                Self::Female
            }
        } else {
            // Choose from minority genders.
            let minority_choice = Rand::in_range(0, 3);
            match minority_choice {
                0 => Self::NonBinary,
                1 => Self::Other,
                2 => Self::PreferNotToSay,
                _ => unreachable!(),
            }
        }
    }
}
