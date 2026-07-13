use std::fmt;

/// Distinct, job-scoped credentials. Implementations must use a cryptographic RNG.
pub struct JobSecrets {
    pub opencode_password: String,
    pub broker_execution_token: String,
    pub broker_control_token: String,
    pub broker_job_token: String,
}

impl JobSecrets {
    pub fn validate(&self) -> Result<(), String> {
        let values = [
            &self.opencode_password,
            &self.broker_execution_token,
            &self.broker_control_token,
            &self.broker_job_token,
        ];
        if values.iter().any(|value| {
            value.len() < 32
                || !value
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_'))
        }) {
            return Err(
                "job credentials must contain at least 32 URL-safe ASCII characters".into(),
            );
        }
        for (index, value) in values.iter().enumerate() {
            if values.iter().skip(index + 1).any(|other| value == other) {
                return Err("job credentials must be distinct".into());
            }
        }
        Ok(())
    }
}

impl fmt::Debug for JobSecrets {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("JobSecrets([REDACTED])")
    }
}

pub trait JobSecretsGenerator: Send {
    fn generate(&mut self) -> Result<JobSecrets, String>;
}

#[cfg(test)]
mod tests {
    use super::*;

    fn secrets(values: [&str; 4]) -> JobSecrets {
        JobSecrets {
            opencode_password: values[0].into(),
            broker_execution_token: values[1].into(),
            broker_control_token: values[2].into(),
            broker_job_token: values[3].into(),
        }
    }

    #[test]
    fn secrets_are_nonempty_distinct_and_redacted() {
        let valid = [
            "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            "cccccccccccccccccccccccccccccccc",
            "dddddddddddddddddddddddddddddddd",
        ];
        assert!(secrets(valid).validate().is_ok());
        assert!(
            secrets([valid[0], valid[1], valid[1], valid[3]])
                .validate()
                .is_err()
        );
        assert!(
            secrets([valid[0], "too-short", valid[2], valid[3]])
                .validate()
                .is_err()
        );
        assert_eq!(format!("{:?}", secrets(valid)), "JobSecrets([REDACTED])");
    }
}
