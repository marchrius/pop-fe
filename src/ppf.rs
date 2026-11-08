//! Safe PPF2 and PPF3 method 2 patch application.
use std::fs::File;
use std::io::{self, Read, Seek, SeekFrom, Write};

#[derive(Debug, thiserror::Error)]
pub enum PatchError {
    #[error("invalid PPF patch: {0}")]
    Invalid(String),
    #[error("PPF validation failed: {0}")]
    Validation(String),
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),
}

pub fn apply(file: &File, patch: &[u8]) -> Result<(), PatchError> {
    if patch.len() < 4 {
        return Err(PatchError::Invalid("header is truncated".into()));
    }
    if patch.ends_with(b".DIZ")
        || (patch.len() >= 6 && &patch[patch.len() - 6..patch.len() - 2] == b".DIZ")
    {
        return Err(PatchError::Invalid(
            "PPF description footer is unsupported".into(),
        ));
    }
    let mut image = file.try_clone()?;
    let image_len = image.metadata()?.len();
    match &patch[..4] {
        b"PPF2" => apply_v2(&mut image, image_len, patch),
        b"PPF3" => apply_v3(&mut image, image_len, patch),
        _ => Err(PatchError::Invalid("unknown PPF version".into())),
    }
}

fn apply_v2(file: &mut File, image_len: u64, patch: &[u8]) -> Result<(), PatchError> {
    if patch.len() < 1084 {
        return Err(PatchError::Invalid("PPF2 header is truncated".into()));
    }
    if u32_at(patch, 56)? as u64 != image_len {
        return Err(PatchError::Validation(
            "disc image size differs from patch".into(),
        ));
    }
    validate_block(file, &patch[60..1084])?;
    let mut pos = 1084;
    while pos < patch.len() {
        let offset = u32_at(patch, pos)? as u64;
        let count = *patch
            .get(pos + 4)
            .ok_or_else(|| PatchError::Invalid("truncated record".into()))?
            as usize;
        pos += 5;
        apply_record(
            file,
            image_len,
            offset,
            patch
                .get(pos..pos + count)
                .ok_or_else(|| PatchError::Invalid("truncated record data".into()))?,
        )?;
        pos += count;
    }
    Ok(())
}

fn apply_v3(file: &mut File, image_len: u64, patch: &[u8]) -> Result<(), PatchError> {
    if patch.len() < 60 {
        return Err(PatchError::Invalid("PPF3 header is truncated".into()));
    }
    if patch[5] != 2 {
        return Err(PatchError::Invalid(
            "only PPF3 method 2 is supported".into(),
        ));
    }
    if patch[56] != 0 {
        return Err(PatchError::Invalid(
            "only PPF3 image type 0 is supported".into(),
        ));
    }
    let undo = patch[58] != 0;
    let mut pos = 60;
    if patch[57] != 0 {
        if patch.len() < 1084 {
            return Err(PatchError::Invalid(
                "PPF3 validation block is truncated".into(),
            ));
        }
        validate_block(file, &patch[60..1084])?;
        pos = 1084;
    }
    while pos < patch.len() {
        let offset = u64_at(patch, pos)?;
        let count = *patch
            .get(pos + 8)
            .ok_or_else(|| PatchError::Invalid("truncated record".into()))?
            as usize;
        pos += 9;
        let data = patch
            .get(pos..pos + count)
            .ok_or_else(|| PatchError::Invalid("truncated record data".into()))?;
        apply_record(file, image_len, offset, data)?;
        pos += count;
        if undo {
            pos = pos
                .checked_add(count)
                .filter(|end| *end <= patch.len())
                .ok_or_else(|| PatchError::Invalid("truncated undo data".into()))?;
        }
    }
    Ok(())
}

fn validate_block(file: &mut File, expected: &[u8]) -> Result<(), PatchError> {
    if expected.len() != 1024 {
        return Err(PatchError::Invalid(
            "validation block has wrong size".into(),
        ));
    }
    file.seek(SeekFrom::Start(0x9320))?;
    let mut actual = [0u8; 1024];
    file.read_exact(&mut actual)
        .map_err(|_| PatchError::Validation("image is too short for validation block".into()))?;
    if actual != expected {
        return Err(PatchError::Validation("disc contents do not match".into()));
    }
    Ok(())
}

fn apply_record(
    file: &mut File,
    image_len: u64,
    offset: u64,
    data: &[u8],
) -> Result<(), PatchError> {
    let end = offset
        .checked_add(data.len() as u64)
        .ok_or_else(|| PatchError::Invalid("record offset overflow".into()))?;
    if end > image_len {
        return Err(PatchError::Invalid("record extends beyond image".into()));
    }
    file.seek(SeekFrom::Start(offset))?;
    file.write_all(data)?;
    Ok(())
}

fn u32_at(data: &[u8], offset: usize) -> Result<u32, PatchError> {
    let bytes: [u8; 4] = data
        .get(offset..offset + 4)
        .ok_or_else(|| PatchError::Invalid("truncated integer".into()))?
        .try_into()
        .map_err(|_| PatchError::Invalid("invalid integer".into()))?;
    Ok(u32::from_le_bytes(bytes))
}

fn u64_at(data: &[u8], offset: usize) -> Result<u64, PatchError> {
    let bytes: [u8; 8] = data
        .get(offset..offset + 8)
        .ok_or_else(|| PatchError::Invalid("truncated integer".into()))?
        .try_into()
        .map_err(|_| PatchError::Invalid("invalid integer".into()))?;
    Ok(u64::from_le_bytes(bytes))
}
