const DEVELOPMENT_VERSION_LABEL: &str = "DEV_BUILD";
const RELEASE_VERSION: Option<&str> = option_env!("GAMETWEAKS_BUILD_VERSION");

pub fn current() -> &'static str {
    RELEASE_VERSION.unwrap_or(DEVELOPMENT_VERSION_LABEL)
}

pub fn is_release_build() -> bool {
    RELEASE_VERSION.is_some()
}

#[cfg(test)]
mod tests {
    use super::{current, is_release_build, DEVELOPMENT_VERSION_LABEL, RELEASE_VERSION};

    #[test]
    fn version_state_matches_the_compile_time_environment() {
        assert_eq!(
            current(),
            RELEASE_VERSION.unwrap_or(DEVELOPMENT_VERSION_LABEL)
        );
        assert_eq!(is_release_build(), RELEASE_VERSION.is_some());
    }
}
