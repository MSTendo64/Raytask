//! SSA optimization passes.

mod cfg_simplify;
mod const_fold;
mod copy_prop;
mod dce;
mod gvn;
mod inline;
mod licm;
mod mem2reg;
mod sccp;
mod strength_reduce;

pub use cfg_simplify::CfgSimplify;
pub use const_fold::ConstFold;
pub use copy_prop::CopyProp;
pub use dce::Dce;
pub use gvn::Gvn;
pub use inline::Inline;
pub use licm::Licm;
pub use mem2reg::Mem2Reg;
pub use sccp::Sccp;
pub use strength_reduce::StrengthReduce;
