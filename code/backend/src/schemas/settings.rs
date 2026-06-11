use serde::Deserialize;

#[derive(Debug, Clone, Deserialize)]
pub struct UpdateSetting {
    pub value: String,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn update_setting_deser() {
        let u: UpdateSetting = serde_json::from_str(r#"{"value":"new_val"}"#).expect("deser");
        assert_eq!(u.value, "new_val");
    }
}
