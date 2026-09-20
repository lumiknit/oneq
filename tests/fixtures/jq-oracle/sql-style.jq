# SQL-style operators: INDEX, JOIN, IN.
{
  indexed: (INDEX(.rows[]; .id | tostring)),
  needle_in_ids: (.rows as $rows | .needle | IN($rows[].id)),
  needle_in_source: (.rows as $rows | .needle as $n | IN($rows[].id; $n)),
  joined: (INDEX(.rows[]; .id | tostring) as $idx | [JOIN($idx; .rows[]; .id | tostring; [.[0].name, .[1].name])])
}
