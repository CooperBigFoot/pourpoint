//! Balanced row-group planning for basin GeoParquet export.

use crate::export::ExportError;

const MIN_BALANCED_ROW_GROUP_SIZE: usize = 4_096;
const TARGET_ROW_GROUP_SIZE: usize = 8_192;

/// Planned row-group sizes for one export file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RowGroupPlan {
    sizes: Vec<usize>,
}

impl RowGroupPlan {
    /// Return the row counts for each planned row group.
    pub fn sizes(&self) -> &[usize] {
        &self.sizes
    }

    /// Consume the plan and return row counts for each planned row group.
    pub fn into_sizes(self) -> Vec<usize> {
        self.sizes
    }
}

/// Plan balanced row-group sizes for a non-empty export.
///
/// # Errors
///
/// | Condition | Error variant |
/// |---|---|
/// | `row_count == 0` | [`ExportError::RowGroupPlanningFailure`] |
/// | balanced groups would violate the 4,096-8,192 policy | [`ExportError::RowGroupPlanningFailure`] |
pub fn plan_row_groups(row_count: usize) -> Result<RowGroupPlan, ExportError> {
    if row_count == 0 {
        return Err(ExportError::RowGroupPlanningFailure {
            row_count,
            reason: "empty exports are not row-group plannable",
        });
    }

    let sizes = if row_count < MIN_BALANCED_ROW_GROUP_SIZE {
        vec![row_count]
    } else {
        let group_count = row_count.div_ceil(TARGET_ROW_GROUP_SIZE);
        let base = row_count / group_count;
        let remainder = row_count % group_count;
        (0..group_count)
            .map(|index| base + usize::from(index < remainder))
            .collect::<Vec<_>>()
    };

    let balanced_groups_are_legal = row_count < MIN_BALANCED_ROW_GROUP_SIZE
        || sizes
            .iter()
            .all(|size| (MIN_BALANCED_ROW_GROUP_SIZE..=TARGET_ROW_GROUP_SIZE).contains(size));
    if balanced_groups_are_legal {
        Ok(RowGroupPlan { sizes })
    } else {
        Err(ExportError::RowGroupPlanningFailure {
            row_count,
            reason: "balanced row groups fall outside the legal size range",
        })
    }
}

#[cfg(test)]
mod export_row_groups_tests {
    use super::*;

    #[test]
    fn export_row_groups_tiny_file_is_one_group() {
        assert_eq!(plan_row_groups(17).unwrap().sizes(), &[17]);
    }

    #[test]
    fn export_row_groups_4096_is_one_group() {
        assert_eq!(plan_row_groups(4_096).unwrap().sizes(), &[4_096]);
    }

    #[test]
    fn export_row_groups_8193_is_two_legal_groups() {
        let sizes = plan_row_groups(8_193).unwrap().into_sizes();
        assert_eq!(sizes, vec![4_097, 4_096]);
        assert!(sizes.iter().all(|size| (4_096..=8_192).contains(size)));
    }

    #[test]
    fn export_row_groups_9000_is_balanced_and_legal() {
        let sizes = plan_row_groups(9_000).unwrap().into_sizes();
        assert_eq!(sizes, vec![4_500, 4_500]);
        assert!(sizes.iter().all(|size| (4_096..=8_192).contains(size)));
    }

    #[test]
    fn export_row_groups_50000_has_no_short_tail() {
        let sizes = plan_row_groups(50_000).unwrap().into_sizes();
        assert_eq!(sizes.iter().sum::<usize>(), 50_000);
        assert_eq!(sizes.len(), 7);
        assert!(sizes.iter().all(|size| (4_096..=8_192).contains(size)));
        assert_eq!(sizes, vec![7_143, 7_143, 7_143, 7_143, 7_143, 7_143, 7_142]);
    }

    #[test]
    fn export_row_groups_zero_rows_are_rejected() {
        assert!(matches!(
            plan_row_groups(0),
            Err(ExportError::RowGroupPlanningFailure { row_count: 0, .. })
        ));
    }
}
