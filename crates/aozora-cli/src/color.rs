use crate::fmt::ColorChoice;
use miette::MietteHandlerOpts;

pub(crate) fn resolve(flag: Option<ColorChoice>, config: Option<ColorChoice>) -> ColorChoice {
    flag.or(config).unwrap_or_default()
}

pub(crate) fn install(choice: ColorChoice) {
    let _ignored = miette::set_hook(Box::new(move |_| {
        let opts = match choice {
            ColorChoice::Always => MietteHandlerOpts::new().color(true),
            ColorChoice::Never => MietteHandlerOpts::new().color(false),
            ColorChoice::Auto => MietteHandlerOpts::new(),
        };
        Box::new(opts.build())
    }));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_flag_beats_the_config_key() {
        assert_eq!(
            resolve(Some(ColorChoice::Always), Some(ColorChoice::Never)),
            ColorChoice::Always
        );
        assert_eq!(
            resolve(Some(ColorChoice::Never), Some(ColorChoice::Always)),
            ColorChoice::Never
        );
    }

    #[test]
    fn the_config_decides_when_the_flag_is_absent() {
        assert_eq!(resolve(None, Some(ColorChoice::Never)), ColorChoice::Never);
        assert_eq!(
            resolve(None, Some(ColorChoice::Always)),
            ColorChoice::Always
        );
    }

    #[test]
    fn nothing_set_is_auto() {
        assert_eq!(resolve(None, None), ColorChoice::Auto);
    }

    #[test]
    fn an_explicit_flag_auto_still_beats_the_config() {
        assert_eq!(
            resolve(Some(ColorChoice::Auto), Some(ColorChoice::Never)),
            ColorChoice::Auto
        );
    }
}
