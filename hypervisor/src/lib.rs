#[macro_use]
extern crate log;

pub mod hypervisor;

/// Shorthand macro to return a new
/// [`TypedError`](a653rs_linux_core::error::TypedError)
///
/// Allows expressing
///
/// ```no_run
/// # use anyhow::anyhow;
/// # use a653rs_linux_core::error::{TypedError, TypedResult, SystemError};
/// # fn main() -> TypedResult<()>{
/// let extra_info = "problem";
/// let problem = anyhow!("a {extra_info} description");
/// return Err(TypedError::new(SystemError::Panic, problem));
/// # }
/// ```
///
/// as a more compact
///
/// ```no_run
/// # use a653rs_linux_core::error::TypedResult;
/// # use a653rs_linux_hypervisor::problem;
/// # fn main() -> TypedResult<()>{
/// # let extra_info = "problem";
/// problem!(Panic, "a {extra_info} description");
/// # }
/// ```
#[macro_export]
macro_rules! problem {
    ($typed_err: expr, $($tail:tt)*) => {{
        #[allow(unused_imports)]
        use ::a653rs_linux_core::error::SystemError::*;
        let problem = ::anyhow::anyhow!($($tail)*);
        return ::a653rs_linux_core::error::TypedResult::Err(
            ::a653rs_linux_core::error::TypedError::new($typed_err, problem)
        );
    }};
}

#[cfg(test)]
mod test {
    use a653rs_linux_core::error::{SystemError, TypedError, TypedResult};
    use anyhow::anyhow;

    fn problem_manual() -> TypedResult<()> {
        let extra_info = "problem";
        let problem = anyhow!("a {extra_info} description");
        return Err(TypedError::new(SystemError::Panic, problem));
    }

    fn problem_macro() -> TypedResult<()> {
        let extra_info = "problem";
        problem!(Panic, "a {extra_info} description");
    }

    #[test]
    fn problem() {
        assert_eq!(
            problem_manual().unwrap_err().to_string(),
            problem_macro().unwrap_err().to_string()
        );
    }
}
