// robot/user_profile.rs - Kullanıcıya Özel Parametre Tuning ve Kişiselleştirme
// Her kullanıcı için ayrı model, parametre ve geçmiş yönetimi

use crate::robot::ml_engine::{MLModel, FeatureVector};
use crate::types::StrategyParams;

pub struct UserProfile {
    pub user_id: String,
    pub model: MLModel,
    pub params: StrategyParams,
    pub history: Vec<FeatureVector>,
}

impl UserProfile {
    pub fn new(user_id: &str) -> Self {
        Self {
            user_id: user_id.to_string(),
            model: MLModel::new(),
            params: StrategyParams::default(),
            history: vec![],
        }
    }
    pub fn update_history(&mut self, fv: FeatureVector) {
        self.history.push(fv);
    }
    pub fn personalize(&mut self) {
        // Dummy: Kullanıcıya özel model eğitimi
        // Note: self.history is Vec<FeatureVector>, fit expects &[(FeatureVector, f64)]
        // Convert if needed: let data: Vec<_> = self.history.iter().map(|f| (f.clone(), f.pnl.unwrap_or(0.0))).collect();
        // self.model.fit(&data);
    }
}
