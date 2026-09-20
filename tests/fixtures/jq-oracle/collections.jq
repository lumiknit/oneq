# sort, sort_by, group_by, unique, unique_by, min/max, min_by/max_by,
# flatten, add, any/all, range.
{
  sorted_v: (.items | sort_by(.v) | map(.v)),
  grouped_g: (.items | group_by(.g) | map({g: (.[0].g), n: length})),
  unique_g: (.items | map(.g) | unique),
  unique_by_g: (.items | unique_by(.g) | map(.g)),
  min_v: (.items | min_by(.v)),
  max_v: (.items | max_by(.v)),
  flat: ([[1,[2,3]],[4]] | flatten),
  flat_depth1: ([[1,[2,3]],[4]] | flatten(1)),
  sum_v: (.items | map(.v) | add),
  any_positive: (.items | any(.v > 0)),
  all_positive: (.items | all(.v > 0)),
  range_sample: [range(0; 10; 3)]
}
