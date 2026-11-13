use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::config::{ClusterMode, TopologySpec};
use crate::error::InstallerError;

/// One deterministic step invoking an external audited `.sh` script.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ScriptStep {
    pub script_name: String,
    pub script_path: PathBuf,
    pub args: Vec<String>,
    pub ssh_target: Option<String>,
    pub ssh_key_path: Option<PathBuf>,
}

/// Ordered provisioning plan generated from a validated [`TopologySpec`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecutionPlan {
    pub mode: ClusterMode,
    pub dry_run: bool,
    pub steps: Vec<ScriptStep>,
}

impl ExecutionPlan {
    /// Builds the full Mode A / Mode B installation plan referencing external `.sh` files.
    ///
    /// # Errors
    ///
    /// Returns [`InstallerError::Validation`] if the topology fails invariants or
    /// [`InstallerError::MissingScript`] if any required `.sh` script is missing on disk.
    pub fn build_install(spec: &TopologySpec) -> Result<Self, InstallerError> {
        spec.validate()?;
        let mut steps = Vec::new();

        steps.push(resolve_step(spec, "00-preflight.sh", preflight_args(spec))?);
        steps.push(resolve_step(spec, "10-hardening.sh", hardening_args(spec))?);

        if spec.mode.includes_server() {
            steps.push(resolve_step(
                spec,
                "20-deps-server.sh",
                deps_server_args(spec),
            )?);
        }

        if spec.mode.includes_boxer() {
            steps.push(resolve_step(
                spec,
                "30-deps-boxer.sh",
                deps_boxer_args(spec),
            )?);
        }

        steps.push(resolve_step(
            spec,
            "40-systemd-units.sh",
            systemd_args(spec),
        )?);
        steps.push(resolve_step(spec, "90-doctor.sh", doctor_args(spec))?);

        Ok(Self {
            mode: spec.mode,
            dry_run: spec.dry_run,
            steps,
        })
    }

    /// Builds a plan to join an additional Boxer VPS onto an existing cluster VLAN.
    ///
    /// # Errors
    ///
    /// Returns [`InstallerError`] if VLAN topology checks fail or scripts are missing.
    pub fn build_join_vlan(spec: &TopologySpec) -> Result<Self, InstallerError> {
        if spec.mode != ClusterMode::Boxer {
            return Err(InstallerError::Validation(
                "join-vlan requires ClusterMode::Boxer".to_owned(),
            ));
        }
        Self::build_install(spec)
    }

    /// Builds a non-destructive diagnostic plan executing `90-doctor.sh`.
    ///
    /// # Errors
    ///
    /// Returns [`InstallerError`] if `90-doctor.sh` cannot be resolved on disk.
    pub fn build_doctor(spec: &TopologySpec) -> Result<Self, InstallerError> {
        if let Some(target) = &spec.ssh_target
            && target.trim().is_empty()
        {
            return Err(InstallerError::Validation(
                "ssh_target must not be blank".to_owned(),
            ));
        }
        Ok(Self {
            mode: spec.mode,
            dry_run: spec.dry_run,
            steps: vec![resolve_step(spec, "90-doctor.sh", doctor_args(spec))?],
        })
    }

    /// Builds an ordered upgrade verification plan (preflight, unit refresh, doctor).
    ///
    /// # Errors
    ///
    /// Returns [`InstallerError`] if validation fails or required scripts are missing.
    pub fn build_upgrade(spec: &TopologySpec) -> Result<Self, InstallerError> {
        spec.validate()?;
        Ok(Self {
            mode: spec.mode,
            dry_run: spec.dry_run,
            steps: vec![
                resolve_step(spec, "00-preflight.sh", preflight_args(spec))?,
                resolve_step(spec, "40-systemd-units.sh", systemd_args(spec))?,
                resolve_step(spec, "90-doctor.sh", doctor_args(spec))?,
            ],
        })
    }
}

fn resolve_step(
    spec: &TopologySpec,
    script_name: &str,
    args: Vec<String>,
) -> Result<ScriptStep, InstallerError> {
    let path = verify_script_exists(&spec.scripts_dir, script_name)?;
    Ok(ScriptStep {
        script_name: script_name.to_owned(),
        script_path: path,
        args,
        ssh_target: spec.ssh_target.clone(),
        ssh_key_path: spec.ssh_key_path.clone(),
    })
}

fn verify_script_exists(scripts_dir: &Path, name: &str) -> Result<PathBuf, InstallerError> {
    if name.contains('/') || name.contains('\\') || name.contains("..") {
        return Err(InstallerError::Validation(format!(
            "unsafe script name: {name}"
        )));
    }
    let path = scripts_dir.join(name);
    if !path.is_file() {
        return Err(InstallerError::MissingScript(path.display().to_string()));
    }
    Ok(path)
}

fn preflight_args(spec: &TopologySpec) -> Vec<String> {
    let mut args = vec!["--role".to_owned(), spec.mode.as_str().to_owned()];
    if spec.dry_run {
        args.push("--dry-run".to_owned());
    }
    args
}

fn hardening_args(spec: &TopologySpec) -> Vec<String> {
    let mut args = vec![
        "--role".to_owned(),
        spec.mode.as_str().to_owned(),
        "--vlan-cidr".to_owned(),
        spec.vlan_cidr.clone(),
    ];
    if spec.dry_run {
        args.push("--dry-run".to_owned());
    }
    args
}

fn deps_server_args(spec: &TopologySpec) -> Vec<String> {
    let mut args = vec![
        "--role".to_owned(),
        spec.mode.as_str().to_owned(),
        "--domain".to_owned(),
        spec.public_domain
            .clone()
            .unwrap_or_else(|| "agents.example.com".to_owned()),
        "--preview-domain".to_owned(),
        spec.preview_domain.clone(),
        "--acme-email".to_owned(),
        spec.acme_email
            .clone()
            .unwrap_or_else(|| "ops@example.com".to_owned()),
        "--vlan-cidr".to_owned(),
        spec.vlan_cidr.clone(),
        "--env-file".to_owned(),
        spec.env_file_path.display().to_string(),
    ];
    if let Some(ip) = spec.vlan_ip {
        args.push("--bind-ip".to_owned());
        args.push(ip.to_string());
    }
    if spec.dry_run {
        args.push("--dry-run".to_owned());
    }
    args
}

fn deps_boxer_args(spec: &TopologySpec) -> Vec<String> {
    let mut args = vec![
        "--runtime".to_owned(),
        spec.runtime.label().to_owned(),
        "--isolation".to_owned(),
        spec.isolation.as_str().to_owned(),
        "--nats-url".to_owned(),
        spec.nats_url.clone(),
        "--max-boxes".to_owned(),
        spec.max_boxes.to_string(),
    ];
    if let Some(ip) = spec.vlan_ip {
        args.push("--vlan-ip".to_owned());
        args.push(ip.to_string());
    }
    if spec.dry_run {
        args.push("--dry-run".to_owned());
    }
    args
}

fn systemd_args(spec: &TopologySpec) -> Vec<String> {
    let mut args = vec![
        "--role".to_owned(),
        spec.mode.as_str().to_owned(),
        "--env-file".to_owned(),
        spec.env_file_path.display().to_string(),
    ];
    if spec.dry_run {
        args.push("--dry-run".to_owned());
    }
    args
}

fn doctor_args(spec: &TopologySpec) -> Vec<String> {
    vec![
        "--role".to_owned(),
        spec.mode.as_str().to_owned(),
        "--env-file".to_owned(),
        spec.env_file_path.display().to_string(),
        "--non-destructive".to_owned(),
    ]
}
