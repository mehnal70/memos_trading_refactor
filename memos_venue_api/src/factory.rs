use std::sync::Arc;

use crate::adapter::VenueAdapter;
use crate::error::Result;
use crate::types::VenueSpec;

/// Venue implementasyonlarını composition katmanına bağlayan factory sözleşmesi.
///
/// Somut market crate'leri core'a bağımlı olmadan bu trait'i implement edebilir;
/// registry yalnızca bu sözleşmeyi ve `VenueSpec` kimliğini bilir.
pub trait VenueFactory: Send + Sync {
    fn build(&self, spec: VenueSpec) -> Result<Arc<dyn VenueAdapter>>;
}
