#include <linux/module.h>
#include <linux/proc_fs.h>
#include <linux/seq_file.h>
#include <linux/utsname.h>

MODULE_LICENSE("GPL");
MODULE_AUTHOR("Nexa OS");
MODULE_DESCRIPTION("Nexa OS /proc/nexa info module");

static int nexa_show(struct seq_file *m, void *v) {
    struct new_utsname *u = utsname();
    seq_printf(m, "name=NexaOS\n");
    seq_printf(m, "version=3.0\n");
    seq_printf(m, "kernel=%s\n", u->release);
    seq_printf(m, "arch=%s\n", u->machine);
    seq_printf(m, "init=C-based\n");
    seq_printf(m, "features=multiuser,httpd,syslogd,crond\n");
    return 0;
}

static int nexa_open(struct inode *inode, struct file *file) {
    return single_open(file, nexa_show, NULL);
}

static const struct proc_ops nexa_ops = {
    .proc_open    = nexa_open,
    .proc_read    = seq_read,
    .proc_lseek   = seq_lseek,
    .proc_release = single_release,
};

static int __init nexa_init(void) {
    proc_create("nexa", 0444, NULL, &nexa_ops);
    printk(KERN_INFO "NexaOS: /proc/nexa registered\n");
    return 0;
}

static void __exit nexa_exit(void) {
    remove_proc_entry("nexa", NULL);
    printk(KERN_INFO "NexaOS: /proc/nexa removed\n");
}

module_init(nexa_init);
module_exit(nexa_exit);
