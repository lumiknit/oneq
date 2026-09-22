[
  (.orders
   | map(select(.customer.account.active)
       | select(.metrics.total >= 100)
       | . + {
           normalized_items: (.items | map(select(.category != "obsolete") | .category) | unique | sort),
           priority: (if .metrics.total >= 300 then "high" elif .metrics.total >= 150 then "normal" else "low" end)
         }))[]
  | {
      id: .id,
      region: .customer.address.region,
      name: .customer.name,
      priority: .priority,
      score: (((.metrics.total * .metrics.quantity) / (.metrics.refunds + 1))
              + (if .priority == "high" then 25 else 0 end)),
      tags: .normalized_items,
      item_count: (.normalized_items | length)
    }
]
| sort_by(-.score, .region, .id)
| group_by(.region)
| map({
    region: .[0].region,
    orders: length,
    total_score: (map(.score) | add),
    top: .[0:3] | map({id, name, score, tags})
  })
| sort_by(-.total_score, .region)
