use std::path::{Component, Path};

use crate::error::{Error, Result};

pub fn path_to_archive(root: &Path, path: &Path) -> Result<String> {
    let relative = path
        .strip_prefix(root)
        .map_err(|_| Error::InvalidPath(path.display().to_string()))?;

    let mut parts = Vec::new();
    for component in relative.components() {
        match component {
            Component::Normal(value) => {
                let value = value.to_str().ok_or_else(|| {
                    Error::InvalidPath(format!("non-UTF-8 path: {}", path.display()))
                })?;
                validate_component(value)?;
                parts.push(value);
            }
            _ => {
                return Err(Error::InvalidPath(format!(
                    "non-relative component in {}",
                    path.display()
                )))
            }
        }
    }

    let result = parts.join("/");
    validate_archive_path(&result)?;
    Ok(result)
}

pub fn validate_archive_path(path: &str) -> Result<()> {
    if path.is_empty() {
        return Err(Error::InvalidPath("empty path".into()));
    }
    if path.starts_with('/') || path.ends_with('/') || path.contains('\0') {
        return Err(Error::InvalidPath(path.into()));
    }

    for component in path.split('/') {
        validate_component(component)?;
    }
    Ok(())
}

pub fn normalize_lookup_path(path: &str) -> Result<String> {
    if path.is_empty() || path == "/" {
        return Ok(String::new());
    }
    let path = path.strip_prefix('/').unwrap_or(path);
    validate_archive_path(path)?;
    Ok(path.to_owned())
}

pub fn parent_path(path: &str) -> &str {
    path.rsplit_once('/').map_or("", |(parent, _)| parent)
}

fn validate_component(component: &str) -> Result<()> {
    if component.is_empty() || component == "." || component == ".." {
        return Err(Error::InvalidPath(component.into()));
    }
    if component.contains('\0') {
        return Err(Error::InvalidPath(component.into()));
    }
    Ok(())
}
