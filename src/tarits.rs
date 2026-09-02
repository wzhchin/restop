use std::path::PathBuf;

pub trait None2NaN<T, F> {
    fn or_nan(&self, f: F) -> String
    where
        F: Fn(&T) -> String;

    fn or_unk(&self, f: F) -> String
    where
        F: Fn(&T) -> String;

    fn or_unspt(&self, f: F) -> String
    where
        F: Fn(&T) -> String;
}

impl<T, F> None2NaN<T, F> for Option<T> {
    fn or_nan(&self, f: F) -> String
    where
        F: Fn(&T) -> String,
    {
        match self {
            Some(v) => f(v),
            None => "N/A".to_owned(),
        }
    }

    fn or_unk(&self, f: F) -> String
    where
        F: Fn(&T) -> String,
    {
        match self {
            Some(v) => f(v),
            None => "Unknown".to_owned(),
        }
    }

    fn or_unspt(&self, f: F) -> String
    where
        F: Fn(&T) -> String,
    {
        match self {
            Some(v) => f(v),
            None => "Unsupported".to_owned(),
        }
    }
}

pub trait None2NaNDef {
    fn or_nan_def(&self) -> &str;
    fn or_unk_def(&self) -> &str;
}

pub trait None2NanString {
    fn or_nan_owned(&self) -> String;
    fn or_unk_owned(&self) -> String;
}

impl None2NaNDef for Option<&str> {
    fn or_nan_def(&self) -> &str {
        match self {
            Some(o) => o,
            None => "N/A",
        }
    }

    fn or_unk_def(&self) -> &str {
        match self {
            Some(o) => o,
            None => "Unknown",
        }
    }
}

impl None2NaNDef for Option<String> {
    fn or_nan_def(&self) -> &str {
        match self {
            Some(o) => o.as_str(),
            None => "N/A",
        }
    }

    fn or_unk_def(&self) -> &str {
        match self {
            Some(o) => o.as_str(),
            None => "Unknown",
        }
    }
}

impl<S> None2NanString for Option<S>
where
    S: ToString,
{
    fn or_nan_owned(&self) -> String {
        match self {
            Some(s) => s.to_string(),
            None => "N/A".to_owned(),
        }
    }

    fn or_unk_owned(&self) -> String {
        match self {
            Some(s) => s.to_string(),
            None => "Unknown".to_owned(),
        }
    }
}

/// Format a 0–100 percent with one decimal and a `%` suffix. Non-finite → `N/A`.
pub fn format_percent_label(percent: f64) -> String {
    if percent.is_finite() {
        format!("{:.1} %", percent)
    } else {
        "N/A".to_owned()
    }
}

/// Format a 0–1 fraction as a percent label. Non-finite values are `N/A`.
pub fn format_fraction_as_percent(fraction: f64) -> String {
    format_percent_label(fraction * 100.0)
}

/// Format a 0–100 percent number with one decimal. Non-finite values are `N/A`.
pub fn format_percent_number(percent: f64) -> String {
    if percent.is_finite() {
        format!("{:.1}", percent)
    } else {
        "N/A".to_owned()
    }
}

pub trait NaNDefault {
    /// Returns the given `default` value if the variable is NaN,
    /// and returns itself otherwise.
    #[must_use]
    fn nan_default(&self, default: Self) -> Self;
}

impl NaNDefault for f64 {
    fn nan_default(&self, default: Self) -> Self {
        if self.is_nan() {
            default
        } else {
            *self
        }
    }
}

impl NaNDefault for f32 {
    fn nan_default(&self, default: Self) -> Self {
        if self.is_nan() {
            default
        } else {
            *self
        }
    }
}

pub trait PathString {
    fn to_filepath(&self) -> String;
    fn to_filename(&self) -> String;
}

impl PathString for PathBuf {
    fn to_filepath(&self) -> String {
        self.to_string_lossy().to_string()
    }

    fn to_filename(&self) -> String {
        match self.file_name() {
            Some(path) => path.to_string_lossy().to_string(),
            None => "<None>".to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{format_fraction_as_percent, format_percent_label, format_percent_number};

    #[test]
    fn fraction_0_5_formats_as_50_percent() {
        assert_eq!(format_fraction_as_percent(0.5), "50.0 %");
        assert!(!format_fraction_as_percent(0.5).contains("0.5 %"));
        assert_eq!(format_percent_label(50.0), "50.0 %");
    }

    #[test]
    fn percent_formatters_never_emit_nan() {
        for label in [
            format_fraction_as_percent(f64::NAN),
            format_fraction_as_percent(f32::NAN as f64),
            format_percent_label(f64::NAN),
            format_percent_label(f32::NAN as f64),
            format_percent_number(f64::NAN),
            format_percent_number(f32::NAN as f64),
        ] {
            assert!(!label.contains("NaN"), "got {label}");
            assert_eq!(label, "N/A");
        }
    }
}
