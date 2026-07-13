use std::fmt;

/// Distinct, job-scoped credentials. Implementations must use a cryptographic RNG.
pub struct JobSecrets {
    opencode_password: String,
    broker_execution_token: String,
    broker_control_token: String,
    broker_job_token: String,
}

pub struct ModelProcessSecrets<'a> {
    pub opencode_password: &'a str,
    pub broker_execution_token: &'a str,
}

pub struct BrokerBootstrapSecrets<'a> {
    pub execution_token: &'a str,
    pub control_token: &'a str,
    pub job_token: &'a str,
}

pub struct BrokerControlSecret<'a>(pub &'a str);

impl JobSecrets {
    pub fn new(
        opencode_password: String,
        broker_execution_token: String,
        broker_control_token: String,
        broker_job_token: String,
    ) -> Result<Self, String> {
        let value = Self {
            opencode_password,
            broker_execution_token,
            broker_control_token,
            broker_job_token,
        };
        value.validate()?;
        Ok(value)
    }

    pub fn model_process(&self) -> ModelProcessSecrets<'_> {
        ModelProcessSecrets {
            opencode_password: &self.opencode_password,
            broker_execution_token: &self.broker_execution_token,
        }
    }

    pub fn broker_bootstrap(&self) -> BrokerBootstrapSecrets<'_> {
        BrokerBootstrapSecrets {
            execution_token: &self.broker_execution_token,
            control_token: &self.broker_control_token,
            job_token: &self.broker_job_token,
        }
    }

    pub fn broker_control(&self) -> BrokerControlSecret<'_> {
        BrokerControlSecret(&self.broker_control_token)
    }

    pub fn redact(&self, value: &str) -> String {
        [
            &self.opencode_password,
            &self.broker_execution_token,
            &self.broker_control_token,
            &self.broker_job_token,
        ]
        .into_iter()
        .fold(value.to_owned(), |text, secret| {
            text.replace(secret, "[REDACTED]")
        })
    }

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

#[derive(Default)]
pub struct SystemJobSecretsGenerator;

impl JobSecretsGenerator for SystemJobSecretsGenerator {
    fn generate(&mut self) -> Result<JobSecrets, String> {
        fn secret() -> Result<String, String> {
            let mut bytes = [0_u8; 32];
            getrandom::fill(&mut bytes).map_err(|error| error.to_string())?;
            Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
        }

        JobSecrets::new(secret()?, secret()?, secret()?, secret()?)
    }
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

    #[test]
    fn system_generator_creates_distinct_256_bit_credentials() {
        let value = SystemJobSecretsGenerator.generate().unwrap();
        value.validate().unwrap();
        for secret in [
            value.opencode_password,
            value.broker_execution_token,
            value.broker_control_token,
            value.broker_job_token,
        ] {
            assert_eq!(secret.len(), 64);
            assert!(secret.bytes().all(|byte| byte.is_ascii_hexdigit()));
        }
    }
}
