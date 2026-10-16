use crate::utils::timedate;
use serde::{Deserialize, Serialize};
use std::cmp::max;
use std::collections::BTreeMap;
use xmlrpc::{Request, Value};

#[derive(Debug, Deserialize, Serialize)]
pub struct Process {
    pub name: String,
    pub state: String,
    pub process_name: String,
    pub pid: i32,
    pub uptime: String,
}

/// Errors that can occur while talking to the supervisor XML-RPC server.
#[derive(Debug, thiserror::Error)]
pub enum SupervisorError {
    /// The request could not be executed (network problem, `<fault>` response, ...).
    #[error("XML-RPC request to supervisor failed: {0}")]
    Rpc(#[from] xmlrpc::Error),

    /// The server answered with a value of an unexpected shape.
    #[error("supervisor returned an unexpected response: {0}")]
    UnexpectedResponse(&'static str),

    /// An entry of the response is missing a required field.
    #[error("supervisor response entry is missing the `{0}` field")]
    MissingField(&'static str),

    /// A field exists but does not have the expected type.
    #[error("field `{0}` of the supervisor response has an unexpected type")]
    UnexpectedFieldType(&'static str),

    /// The timestamps of a process entry cannot be interpreted.
    #[error("invalid timestamps in supervisor response: start={start}, stop={stop}, now={now}")]
    InvalidTimestamps { start: i64, stop: i64, now: i64 },
}

#[derive(Debug, Clone)]
pub struct SupervisorService {
    server_url: String,
}

impl SupervisorService {
    pub fn new(server_url: String) -> Self {
        SupervisorService { server_url }
    }

    /// Lists every process known to supervisor.
    pub fn process_list(&self) -> Result<Vec<Process>, SupervisorError> {
        let request = Request::new("supervisor.getAllProcessInfo");
        let response = request.call_url(&self.server_url)?;

        response
            .as_array()
            .ok_or(SupervisorError::UnexpectedResponse(
                "expected an array of processes",
            ))?
            .iter()
            .map(Self::parse_process)
            .collect()
    }

    /// Starts a single supervisor process group.
    pub fn start_process(&self, process_name: String) -> Result<(), SupervisorError> {
        let request = Request::new("supervisor.startProcessGroup").arg(process_name);
        request.call_url(&self.server_url)?;
        Ok(())
    }

    /// Starts every supervisor process group.
    pub fn start_all_process(&self) -> Result<(), SupervisorError> {
        let request = Request::new("supervisor.startAllProcesses");
        request.call_url(&self.server_url)?;
        Ok(())
    }

    /// Stops a single supervisor process group.
    pub fn stop_process(&self, process_name: String) -> Result<(), SupervisorError> {
        let request = Request::new("supervisor.stopProcessGroup").arg(process_name);
        request.call_url(&self.server_url)?;
        Ok(())
    }

    /// Stops every supervisor process group.
    pub fn stop_all_process(&self) -> Result<(), SupervisorError> {
        let request = Request::new("supervisor.stopAllProcesses");
        request.call_url(&self.server_url)?;
        Ok(())
    }

    /// Asks supervisor to reload its configuration.
    pub fn reload_supervisor(&self) -> Result<(), SupervisorError> {
        let request = Request::new("supervisor.reloadConfig");
        request.call_url(&self.server_url)?;
        Ok(())
    }

    /// Converts a single `getAllProcessInfo` entry into a [`Process`].
    fn parse_process(value: &Value) -> Result<Process, SupervisorError> {
        let entry = value
            .as_struct()
            .ok_or(SupervisorError::UnexpectedResponse(
                "expected every process entry to be a struct",
            ))?;

        let state = Self::string_field(entry, "statename")?;
        let start_time = Self::i64_field(entry, "start")?;
        let stop_time = Self::i64_field(entry, "stop")?;
        let now_time = Self::i64_field(entry, "now")?;
        let uptime = timedate::diff_for_humans(max(start_time, stop_time), now_time).ok_or(
            SupervisorError::InvalidTimestamps {
                start: start_time,
                stop: stop_time,
                now: now_time,
            },
        )?;

        Ok(Process {
            name: Self::string_field(entry, "group")?,
            state,
            process_name: Self::string_field(entry, "name")?,
            pid: Self::i32_field(entry, "pid")?,
            uptime,
        })
    }

    /// Fetches a required field of a response entry.
    fn field<'a>(
        entry: &'a BTreeMap<String, Value>,
        field: &'static str,
    ) -> Result<&'a Value, SupervisorError> {
        entry.get(field).ok_or(SupervisorError::MissingField(field))
    }

    fn string_field(
        entry: &BTreeMap<String, Value>,
        field: &'static str,
    ) -> Result<String, SupervisorError> {
        Self::field(entry, field)?
            .as_str()
            .map(str::to_owned)
            .ok_or(SupervisorError::UnexpectedFieldType(field))
    }

    fn i64_field(
        entry: &BTreeMap<String, Value>,
        field: &'static str,
    ) -> Result<i64, SupervisorError> {
        Self::field(entry, field)?
            .as_i64()
            .ok_or(SupervisorError::UnexpectedFieldType(field))
    }

    fn i32_field(
        entry: &BTreeMap<String, Value>,
        field: &'static str,
    ) -> Result<i32, SupervisorError> {
        Self::field(entry, field)?
            .as_i32()
            .ok_or(SupervisorError::UnexpectedFieldType(field))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_entry() -> BTreeMap<String, Value> {
        let mut entry = BTreeMap::new();
        entry.insert("group".to_string(), Value::String("nginx".to_string()));
        entry.insert(
            "name".to_string(),
            Value::String("nginx:worker".to_string()),
        );
        entry.insert(
            "statename".to_string(),
            Value::String("RUNNING".to_string()),
        );
        entry.insert("pid".to_string(), Value::Int(1234));
        entry.insert("start".to_string(), Value::Int64(1_700_000_000));
        entry.insert("stop".to_string(), Value::Int64(0));
        entry.insert("now".to_string(), Value::Int64(1_700_003_600));
        entry
    }

    #[test]
    fn parse_process_reads_every_field() {
        let process =
            SupervisorService::parse_process(&Value::Struct(valid_entry())).expect("valid entry");

        assert_eq!(process.name, "nginx");
        assert_eq!(process.process_name, "nginx:worker");
        assert_eq!(process.state, "RUNNING");
        assert_eq!(process.pid, 1234);
        assert!(process.uptime.starts_with("1 hour ago"));
    }

    #[test]
    fn parse_process_reports_missing_fields() {
        let mut entry = valid_entry();
        entry.remove("statename");

        let error = SupervisorService::parse_process(&Value::Struct(entry))
            .expect_err("statename is missing");
        assert!(matches!(error, SupervisorError::MissingField("statename")));
    }

    #[test]
    fn parse_process_reports_wrong_field_types() {
        let mut entry = valid_entry();
        entry.insert("pid".to_string(), Value::String("1234".to_string()));

        let error =
            SupervisorService::parse_process(&Value::Struct(entry)).expect_err("pid is a string");
        assert!(matches!(error, SupervisorError::UnexpectedFieldType("pid")));
    }

    #[test]
    fn parse_process_rejects_non_struct_entries() {
        let error = SupervisorService::parse_process(&Value::String("oops".to_string()))
            .expect_err("entry is not a struct");
        assert!(matches!(error, SupervisorError::UnexpectedResponse(_)));
    }
}
