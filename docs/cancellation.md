# Pipeline 协作取消

JSON API v1.5 为导入管线增加了可送达的协作取消。目标是解决一个具体死锁式等待：普通 Dart worker 串行执行同步 FFI，长时间 `pipeline.step` 运行时，排在同一消息队列后的 `pipeline.cancel` 永远要等它结束。

## 线程与锁模型

- 普通 `PerchWorker` 请求仍按序执行，保持句柄操作顺序确定。
- `PipelineHandle.cancel` 使用一次性的控制 isolate 直接调用同一进程中的 `pipeline.cancel`。
- Rust 管线对象把取消集合放在执行 mutex 之外。`pipeline.cancel` 先登记信号；若执行锁空闲，同时立即撤出排队、运行或耗尽任务；若锁正被 `step` 持有，不等待该锁。
- `pipeline.step` 在文件读取、格式读取、解析/抽取、索引构建和产物保存之间检查信号。观察到信号后从调度器移除任务并返回 `outcome: "cancelled"`。

取消是检查点式而非逐字符抢占。一次已经开始的文件读取、EPUB/TXT 解析或索引构建会先完成当前子操作；当前 Linux 100 MiB 样本的完整 refine/index p50 分别约 79/178ms，因此暂不为这些内部循环增加原子检查成本。若移动真机出现明显长尾，再把 token 下沉到解码、章节循环和索引分片循环。

## 持久化语义

取消不删除源文件或既有产物，也不把任务标为失败：

- refine 取消后，落盘状态保持 `refining`；可能已经存在幂等的 `refinement.json`。
- index 取消后，落盘状态保持 `indexing`；可能已经写入尚未提交 Ready 的索引产物。
- 重开后仍按现有恢复规则从该阶段重做；显式再次 enqueue/retry 会清掉旧取消信号。

`pipeline.cancel` 的 `cancelled: true` 表示取消请求已登记，不代表它一定先于最终提交点到达。调用方需要以对应 `pipeline.step` 的 `cancelled` 或成功结果作为最终竞态结论。

## 执行活动快照

`pipeline.progress` 使用独立活动锁，不等待 `pipeline.step` 持有的执行锁。它返回当前 book、refine/index 阶段、`readingSource` / `refining` / `buildingIndex` / `savingIndex` 等粗粒度 checkpoint，以及取样时是否已登记取消。无在途任务时返回 `active:null`。

Dart `PipelineHandle.progress` 与 cancel 共用短生命周期控制 isolate，因此普通 worker 正在同步 step 时仍可轮询。checkpoint 表示当前子操作，不是百分比或耗时估计；解析和索引内部循环仍没有更细粒度计数。

## Context 关闭

原生 `context.close` 会先从根注册表摘除 context，因此后续请求立即失败；然后对所属 pipeline 设置全局停止信号，等待所有已接受的 context 调用归还内部租约后才返回。在途 step 在下一安全检查点返回 `cancelled`；已开始的单个文件读取、解析或建索引子操作仍先完成。

Dart worker 的 context 创建已移到受跟踪的短生命周期 isolate；即使常驻 worker 在创建期间被强制终止，主 isolate 仍能获得编号，`dispose` 会等待创建完成并通过上述排空语义成组回收句柄。

可量化百分比仍不在当前契约中：粗粒度 checkpoint 已满足可观察进度的验收，只有真实 UI 需要百分比时才定义可比较工作量与采样成本。
