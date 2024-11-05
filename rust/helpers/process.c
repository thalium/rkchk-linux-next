#include <linux/sched/signal.h>
#include <linux/sched.h>

struct task_struct *rust_helper_next_task(const struct task_struct *p)
{
	return next_task(p);
}