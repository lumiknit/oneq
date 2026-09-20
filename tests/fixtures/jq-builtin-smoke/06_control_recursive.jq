{
  any0: ([false,true,false] | any),
  any1: ([1,2,3] | any(. > 2)),
  any2: (any(1,2,3; . > 2)),
  all0: ([true,true] | all),
  all1: ([1,2,3] | all(. > 0)),
  all2: (all(1,2,3; . > 0)),
  while_: (1 | [while(. < 10; . * 2)]),
  until: (1 | until(. >= 10; . * 2)),
  repeat: (1 | [limit(3; repeat(. * 2))]),
  recurse: ({a:{a:null}} | [recurse(.a?; . != null)]),
  recurse1: ({a:{a:null}} | [recurse(.a?; . != null)]),
  recurse2: (1 | [recurse(.+1; . < 4)]),
  walk: ({a:[2,1]} | walk(if type=="array" then sort else . end))
}
