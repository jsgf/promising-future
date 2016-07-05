var searchIndex = {};
searchIndex["promising_future"] = {"doc":"Futures and Promises\n====================","items":[[3,"ThreadSpawner","promising_future","An implementation of `Spawner` that creates normal `std::thread` threads.",null,null],[3,"FutureStream","","Stream of multiple `Future`s",null,null],[3,"FutureStreamIter","","Iterator for completed `Future`s in a `FutureStream`. The iterator incrementally returns values\nfrom resolved `Future`s, blocking while there are no unresolved `Future`s. `Future`s which\nresolve to no value are discarded.",null,null],[3,"FutureStreamWaiter","","Waiter for `Future`s in a `FutureStream`.",null,null],[3,"Future","","An undetermined value.",null,null],[3,"Promise","","A box for resolving a `Future`.",null,null],[3,"FutureIter","","Blocking iterator for the value of a `Future`. Returns either 0 or 1 values.",null,null],[4,"Pollresult","","Result of calling `Future.poll()`.",null,null],[13,"Unresolved","","`Future` is not yet resolved; returns the `Future` for further use.",0,null],[13,"Resolved","","`Future` has been resolved, and may or may not have a value. The `Future` has been consumed.",0,null],[5,"any","","Return first available `Future` from an iterator of `Future`s.",null,{"inputs":[{"name":"i"}],"output":{"name":"option"}}],[5,"all","","Return a Future of all values in an iterator of `Future`s.",null,{"inputs":[{"name":"i"}],"output":{"name":"future"}}],[5,"all_with","","Return a Future of all values in an iterator of `Future`s.",null,{"inputs":[{"name":"i"},{"name":"s"}],"output":{"name":"future"}}],[5,"future_promise","","Construct a `Future`/`Promise` pair.",null,null],[11,"spawn","","",1,{"inputs":[{"name":"threadspawner"},{"name":"f"}],"output":null}],[11,"spawn","threadpool","",2,{"inputs":[{"name":"threadpool"},{"name":"f"}],"output":null}],[11,"clone","promising_future","",3,{"inputs":[{"name":"futurestream"}],"output":{"name":"futurestream"}}],[11,"new","","",3,{"inputs":[],"output":{"name":"futurestream"}}],[11,"add","","Add a `Future` to the stream.",3,{"inputs":[{"name":"futurestream"},{"name":"future"}],"output":null}],[11,"outstanding","","Return number of outstanding `Future`s.",3,{"inputs":[{"name":"futurestream"}],"output":{"name":"usize"}}],[11,"waiter","","Return a singleton `FutureStreamWaiter`. If one already exists, block until it is released.",3,{"inputs":[{"name":"futurestream"}],"output":{"name":"futurestreamwaiter"}}],[11,"try_waiter","","Return a singleton `FutureStreamWaiter`. Returns `None` if one already exists.",3,{"inputs":[{"name":"futurestream"}],"output":{"name":"option"}}],[11,"poll","","Return a resolved `Future` if any, but don&#39;t wait for more to resolve.",3,{"inputs":[{"name":"futurestream"}],"output":{"name":"option"}}],[11,"wait","","Return resolved `Future`s. Blocks if there are outstanding `Futures` which are not yet\nresolved. Returns `None` when there are no more outstanding `Future`s.",3,{"inputs":[{"name":"futurestream"}],"output":{"name":"option"}}],[11,"wait","","Return resolved `Future`s. Blocks if there are outstanding `Futures` which are not yet\nresolved. Returns `None` when there are no more outstanding `Future`s.",4,{"inputs":[{"name":"futurestreamwaiter"}],"output":{"name":"option"}}],[11,"poll","","Return next resolved `Future`, but don&#39;t wait for more to resolve.",4,{"inputs":[{"name":"futurestreamwaiter"}],"output":{"name":"option"}}],[11,"drop","","",4,{"inputs":[{"name":"futurestreamwaiter"}],"output":null}],[11,"into_iter","","",4,{"inputs":[{"name":"futurestreamwaiter"}],"output":{"name":"intoiter"}}],[11,"next","","",5,{"inputs":[{"name":"futurestreamiter"}],"output":{"name":"option"}}],[11,"from_iter","","",3,{"inputs":[{"name":"i"}],"output":{"name":"self"}}],[8,"Spawner","","A trait for spawning threads.",null,null],[10,"spawn","","Spawn a thread to run function `f`.",6,{"inputs":[{"name":"spawner"},{"name":"f"}],"output":null}],[11,"fmt","","",0,{"inputs":[{"name":"pollresult"},{"name":"formatter"}],"output":{"name":"result"}}],[11,"from","","",0,{"inputs":[{"name":"pollresult"}],"output":{"name":"self"}}],[11,"fmt","","",7,{"inputs":[{"name":"future"},{"name":"formatter"}],"output":{"name":"result"}}],[11,"fmt","","",8,{"inputs":[{"name":"promise"},{"name":"formatter"}],"output":{"name":"result"}}],[11,"with_value","","Construct an already resolved `Future` with a value. It is equivalent to a `Future` whose\n`Promise` has already been fulfilled.",7,{"inputs":[{"name":"t"}],"output":{"name":"future"}}],[11,"never","","Construct a resolved `Future` which will never have a value; it is equivalent to a `Future`\nwhose `Promise` completed unfulfilled.",7,{"inputs":[],"output":{"name":"future"}}],[11,"poll","","Test to see if the `Future` is resolved yet.",7,{"inputs":[{"name":"future"}],"output":{"name":"pollresult"}}],[11,"poll_ref","","Non-destructively poll a `Future`s value.",7,{"inputs":[{"name":"future"}],"output":{"name":"result"}}],[11,"value","","Block until the `Future` is resolved.",7,{"inputs":[{"name":"future"}],"output":{"name":"option"}}],[11,"value_ref","","Non-destructively get a reference to the `Future`s value.",7,{"inputs":[{"name":"future"}],"output":{"name":"option"}}],[11,"then_opt","","Set a synchronous callback to run in the Promise&#39;s context.",7,{"inputs":[{"name":"future"},{"name":"f"}],"output":{"name":"future"}}],[11,"then","","Set synchronous callback",7,{"inputs":[{"name":"future"},{"name":"f"}],"output":{"name":"future"}}],[11,"callback","","Set a callback to run in the `Promise`&#39;s context.",7,{"inputs":[{"name":"future"},{"name":"f"}],"output":{"name":"future"}}],[11,"callback_unit","","Set a callback which returns `()`",7,{"inputs":[{"name":"future"},{"name":"f"}],"output":null}],[11,"chain","","Chain two `Future`s.",7,{"inputs":[{"name":"future"},{"name":"f"}],"output":{"name":"future"}}],[11,"chain_with","","As with `chain`, but pass a `Spawner` to control how the thread is created.",7,{"inputs":[{"name":"future"},{"name":"f"},{"name":"s"}],"output":{"name":"future"}}],[11,"from","","",7,{"inputs":[{"name":"option"}],"output":{"name":"future"}}],[11,"into_iter","","",7,{"inputs":[{"name":"future"}],"output":{"name":"intoiter"}}],[11,"next","","",9,{"inputs":[{"name":"futureiter"}],"output":{"name":"option"}}],[11,"set","","Fulfill the `Promise` by resolving the corresponding `Future` with a value.",8,{"inputs":[{"name":"promise"},{"name":"t"}],"output":null}],[11,"canceled","","Return true if the corresponding `Future` no longer exists, and so any value set would be\ndiscarded.",8,{"inputs":[{"name":"promise"}],"output":{"name":"bool"}}]],"paths":[[4,"Pollresult"],[3,"ThreadSpawner"],[3,"ThreadPool"],[3,"FutureStream"],[3,"FutureStreamWaiter"],[3,"FutureStreamIter"],[8,"Spawner"],[3,"Future"],[3,"Promise"],[3,"FutureIter"]]};
searchIndex["threadpool"] = {"doc":"A thread pool used to execute functions in parallel.","items":[[3,"ThreadPool","threadpool","Abstraction of a thread pool for basic parallelism.",null,null],[11,"clone","","",0,{"inputs":[{"name":"threadpool"}],"output":{"name":"threadpool"}}],[11,"new","","Spawns a new thread pool with `num_threads` threads.",0,{"inputs":[{"name":"usize"}],"output":{"name":"threadpool"}}],[11,"new_with_name","","Spawns a new thread pool with `num_threads` threads. Each thread will have the\n[name][thread name] `name`.",0,{"inputs":[{"name":"string"},{"name":"usize"}],"output":{"name":"threadpool"}}],[11,"execute","","Executes the function `job` on a thread in the pool.",0,{"inputs":[{"name":"threadpool"},{"name":"f"}],"output":null}],[11,"active_count","","Returns the number of currently active threads.",0,{"inputs":[{"name":"threadpool"}],"output":{"name":"usize"}}],[11,"max_count","","Returns the number of created threads",0,{"inputs":[{"name":"threadpool"}],"output":{"name":"usize"}}],[11,"panic_count","","Returns the number of panicked threads over the lifetime of the pool.",0,{"inputs":[{"name":"threadpool"}],"output":{"name":"usize"}}],[11,"set_threads","","**Deprecated: Use `ThreadPool::set_num_threads`**",0,{"inputs":[{"name":"threadpool"},{"name":"usize"}],"output":null}],[11,"set_num_threads","","Sets the number of worker-threads to use as `num_threads`.\nCan be used to change the threadpool size during runtime.\nWill not abort already running or waiting threads.",0,{"inputs":[{"name":"threadpool"},{"name":"usize"}],"output":null}],[11,"fmt","","",0,{"inputs":[{"name":"threadpool"},{"name":"formatter"}],"output":{"name":"result"}}]],"paths":[[3,"ThreadPool"]]};
initSearch(searchIndex);