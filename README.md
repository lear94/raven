
**What is RAVEN?**
------------

**RAVEN** is a lightweight package manager for **[GNU/Linux](https://en.wikipedia.org/wiki/GNU/Linux)** systems.

----------

### **Installation**
------------

To install RAVEN, run the following commands:

```bash
git clone https://github.com/lear94/raven.git
cd raven
make install
```

### **Usage**
------------------------

- **Install a package:**

  ```bash
  raven -I name-version.rvn
  raven --install name-version.rvn
  ```

- **Uninstall a package:**

  ```bash
  raven -R name-version
  raven --remove name-version
  ```

- **List all installed packages:**

  ```bash
  raven -L
  raven --list
  ```

- **Show help:**

  ```bash
  raven -H
  raven --help
  ```

- **Show the installed RAVEN version:**

  ```bash
  raven -V
  raven --version
  ```

### **Optional Arguments**
----------------------

- **Suppress output during compilation:**

  ```bash
  raven -q
  raven --quiet
  ```
  Example:
  ```bash
  raven --quiet --install name-version.rvn
  ```

- **Run tests after compiling a package:**

  ```bash
  raven -c
  raven --check
  ```

- **Clean up after installing a package:**

  ```bash
  raven -c
  raven --clean
  ```

- **Reinstall a package:**

  ```bash
  raven -e
  raven --reinstall
  ```
  Example:
  ```bash
  raven --reinstall --install name-version.rvn
  ```

### **Creating a Package**
----------------------

**Template for package creation:**

```bash
#!/bin/bash

NAME=''
VERSION=1.0
FILES=('')
SHA256SUMS=('')
DEPENDS=('')
LICENSE=''

PREPARE()
{
  :
}

BUILD()
{
  :
}

CHECK()
{
  :
}

MERGE()
{
  :
}
```
