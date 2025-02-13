/**
 * @enum {string}
 */
const LogLevel = {
  DEBUG: "DEBUG",
  INFO: "INFO",
  WARN: "WARN",
  ERROR: "ERROR",
};

/**
 * @class Logger
 * Utility class for structured logging
 */
class Logger {
  /**
   * @param {string} context - The context/component name for this logger
   * @param {LogLevel} [minLevel=LogLevel.INFO] - Minimum level to log
   */
  constructor(context, minLevel = LogLevel.INFO) {
    this.context = context;
    this.minLevel = minLevel;
    this.levels = {
      [LogLevel.DEBUG]: 0,
      [LogLevel.INFO]: 1,
      [LogLevel.WARN]: 2,
      [LogLevel.ERROR]: 3,
    };
  }

  /**
   * Format a log message
   * @private
   * @param {LogLevel} level
   * @param {string} message
   * @returns {string}
   */
  format(level, message) {
    const timestamp = new Date().toISOString();
    return `[${timestamp}] [${level}] [${this.context}] ${message}`;
  }

  /**
   * @private
   * @param {LogLevel} level
   * @returns {boolean}
   */
  shouldLog(level) {
    return this.levels[level] >= this.levels[this.minLevel];
  }

  debug(message, ...args) {
    if (this.shouldLog(LogLevel.DEBUG)) {
      console.debug(this.format(LogLevel.DEBUG, message), ...args);
    }
  }

  info(message, ...args) {
    if (this.shouldLog(LogLevel.INFO)) {
      console.info(this.format(LogLevel.INFO, message), ...args);
    }
  }

  warn(message, ...args) {
    if (this.shouldLog(LogLevel.WARN)) {
      console.warn(this.format(LogLevel.WARN, message), ...args);
    }
  }

  error(message, ...args) {
    if (this.shouldLog(LogLevel.ERROR)) {
      console.error(this.format(LogLevel.ERROR, message), ...args);
    }
  }
}

/**
 * @template T
 * @typedef {Object<string, any[]>} AccumulatedData
 * A dictionary of arrays where each key represents a type of data to accumulate
 * and the value is an array of that data type
 */

/**
 * @template T
 * @class TreeNode
 * A tree node that can accumulate different types of data along active paths
 */
class TreeNode {
  /**
   * @param {AccumulatedData<T>} nodeData - Dictionary of arrays containing different types of data for this node
   * @param {TreeNode<T>} [parent] - Optional parent node reference
   */
  constructor(nodeData, parent = null) {
    // Create a logger instance for this class
    this.logger = new Logger("TreeNode");

    if (typeof nodeData !== "object") {
      this.logger.error("Invalid nodeData:", nodeData);
      throw new Error(
        "nodeData must be an object mapping string keys to arrays"
      );
    }

    this.logger.debug("Creating new node", { nodeData });
    this.nodeData = nodeData;
    /** @type {TreeNode<T>[]} */
    this.children = [];
    /** @type {number} - Index of the active child (-1 if none) */
    this.activeChildIndex = -1;
    /** @type {TreeNode<T>} - Reference to parent node */
    this.parent = parent;
  }

  /**
   * Add a child node.
   * The child node should have its own htmlElements array (with its edge HTML included if desired).
   * @param {TreeNode<T>} child - Child node to add.
   * @returns {TreeNode<T>} - The added child node.
   */
  addChild(child) {
    this.logger.debug("Adding child", {
      parentElements: this.nodeData.html,
      childElements: child.nodeData.html,
    });

    // Set parent reference
    child.parent = this;

    // Add to children array
    this.children.push(child);

    // Set as active child if first child
    if (this.activeChildIndex === -1) {
      this.activeChildIndex = 0;
      this.logger.debug("Set first child as active");
    }

    return child;
  }

  /**
   * Remove a child node.
   * @param {TreeNode<T>} child - Child node to remove.
   * @returns {boolean} - True if the child was removed.
   */
  removeChild(child) {
    const index = this.children.indexOf(child);
    if (index === -1) return false;

    // Clear parent reference
    child.parent = null;

    // Remove from children array
    this.children.splice(index, 1);

    // Update active child index
    if (this.activeChildIndex === index) {
      this.activeChildIndex = this.children.length > 0 ? 0 : -1;
    } else if (this.activeChildIndex > index) {
      this.activeChildIndex--;
    }

    return true;
  }

  /**
   * Get all children nodes.
   * @returns {TreeNode<T>[]} - Array of child nodes.
   */
  getChildren() {
    return this.children.slice();
  }

  /**
   * Update this node's accumulated data for a specific key
   * @param {string} key - The key for the type of data to update
   * @param {any[]} values - New array of values for this data type
   */
  updateNodeData(key, values) {
    if (!Array.isArray(values)) {
      throw new Error(`Values for key ${key} must be an array`);
    }
    this.logger.debug("Updating node data", {
      key,
      oldValues: this.nodeData[key],
      newValues: values,
    });
    this.nodeData[key] = values;
  }

  /**
   * Get accumulated data of a specific type along the active path
   * @param {string} key - The key for the type of data to accumulate
   * @returns {any[]} - Concatenated array of values along the active path
   */
  getActivePathData(key) {
    this.logger.debug(`Getting active path data for key: ${key}`);
    let result = this.nodeData[key] || [];
    const activeChild = this.getActiveChild();
    if (activeChild) {
      this.logger.debug("Including active child data", {
        childData: activeChild.nodeData[key],
      });
      result = result.concat(activeChild.getActivePathData(key));
    }
    return result;
  }

  /**
   * Get all accumulated data along the active path
   * @returns {AccumulatedData<T>} - Dictionary of accumulated arrays for each data type
   */
  getAllActivePathData() {
    const result = {};
    // Start with this node's data
    for (const [key, values] of Object.entries(this.nodeData)) {
      result[key] = [...values];
    }

    // Recursively get active child's data
    const activeChild = this.getActiveChild();
    if (activeChild) {
      const childData = activeChild.getAllActivePathData();
      // Merge child data with current data
      for (const [key, values] of Object.entries(childData)) {
        if (result[key]) {
          result[key] = result[key].concat(values);
        } else {
          result[key] = values;
        }
      }
    }
    return result;
  }

  /**
   * Update this node's HTML elements.
   * @param {string[]} htmlElements - New array of HTML elements.
   */
  updateHtmlElements(htmlElements) {
    if (!Array.isArray(htmlElements)) {
      throw new Error("htmlElements must be an array of strings");
    }
    this.logger.debug("Updating HTML elements", {
      oldElements: this.nodeData.html,
      newElements: htmlElements,
    });
    this.nodeData.html = htmlElements;
  }

  /**
   * Set the active child of this node.
   * @param {TreeNode<T>} child - Child node to set as active.
   */
  setActiveChild(child) {
    const index = this.children.indexOf(child);
    if (index === -1) {
      this.logger.error("Failed to set active child - not found", {
        childElements: child.nodeData.html,
      });
      throw new Error("Child not found in this node.");
    }

    this.logger.info("Setting active child", {
      previousIndex: this.activeChildIndex,
      newIndex: index,
    });

    this.activeChildIndex = index;
  }

  /**
   * Get the active child of this node.
   * @returns {TreeNode<T> | null}
   */
  getActiveChild() {
    if (this.activeChildIndex === -1 && this.children.length > 0) {
      this.activeChildIndex = 0;
    }
    return this.activeChildIndex !== -1
      ? this.children[this.activeChildIndex]
      : null;
  }

  /**
   * Get the root node by traversing up the tree
   * @returns {TreeNode<T>}
   */
  getRoot() {
    let current = this;
    while (current.parent) {
      current = current.parent;
    }
    return current;
  }

  /**
   * Get the path from root to this node
   * @returns {TreeNode<T>[]}
   */
  getPathFromRoot() {
    const path = [];
    let current = this;
    while (current) {
      path.unshift(current);
      current = current.parent;
    }
    return path;
  }

  /**
   * Recursively render the HTML along the active path.
   * It concatenates this node's HTML (joined from the htmlElements array)
   * and then the active child's rendered path.
   * @returns {string} - Concatenated HTML.
   */
  renderActivePath() {
    this.logger.debug("Rendering active path");
    let result = this.nodeData.html.join("");
    const activeChild = this.getActiveChild();
    if (activeChild) {
      this.logger.debug("Including active child in path", {
        childElements: activeChild.nodeData.html,
      });
      result += activeChild.renderActivePath();
    }
    return result;
  }

  /**
   * Generate a string visualization of the tree.
   * @param {string} prefix - Prefix for the current line.
   * @param {boolean} isLast - Is this the last child of its parent.
   * @returns {string} - String representation of the tree.
   */
  visualize(prefix = "", isLast = true) {
    let result =
      prefix +
      (prefix ? (isLast ? "└── " : "├── ") : "") +
      this.nodeData.html.join("") +
      "\n";
    for (let i = 0; i < this.children.length; i++) {
      const child = this.children[i];
      const newPrefix = prefix + (isLast ? "    " : "│   ");
      const isLastChild = i === this.children.length - 1;
      result += child.visualize(newPrefix, isLastChild);
    }
    return result;
  }

  /**
   * Generate a string visualization of the active path.
   * @param {string} prefix - Prefix for the current line.
   * @returns {string} - String representation of the active path.
   */
  visualizeActivePath(prefix = "") {
    let result =
      prefix + (prefix ? "└── " : "") + this.nodeData.html.join("") + "\n";
    const activeChild = this.getActiveChild();
    if (activeChild) {
      result += activeChild.visualizeActivePath(prefix + "    ");
    }
    return result;
  }
}

// Unit Tests
class TestRunner {
  constructor() {
    this.tests = [];
    this.passed = 0;
    this.failed = 0;
  }

  test(name, fn) {
    this.tests.push({ name, fn });
  }

  assert(condition, message) {
    if (!condition) {
      throw new Error(message);
    }
  }

  run() {
    console.log("Running tests...\n");
    this.tests.forEach(({ name, fn }) => {
      try {
        fn();
        console.log(`✅ ${name}`);
        this.passed++;
      } catch (error) {
        console.error(`❌ ${name}`);
        console.error(`   ${error.message}`);
        this.failed++;
      }
    });
    console.log(`\nResults: ${this.passed} passed, ${this.failed} failed`);
  }
}

const runner = new TestRunner();

// Test: Create node with HTML content
runner.test("Create node with HTML content", () => {
  const node = new TreeNode({ html: ["<div>Test</div>"] });
  console.log("\nTree visualization:");
  console.log(node.visualize());
  runner.assert(
    node.nodeData.html.join("") === "<div>Test</div>",
    "Node HTML content mismatch"
  );
  runner.assert(node.children.length === 0, "New node should have no children");
});

// Test: Add child
runner.test("Add child", () => {
  const parent = new TreeNode({ html: ["<div>Parent</div>"] });
  const child = new TreeNode({
    html: ["<span>Edge Parent-Child</span>", "<div>Child</div>"],
  });
  parent.addChild(child);
  console.log("\nTree visualization:");
  console.log(parent.visualize());
  runner.assert(parent.children.length === 1, "Parent should have one child");
});

// Test: Remove child
runner.test("Remove child", () => {
  const parent = new TreeNode({ html: ["<div>Parent</div>"] });
  const child = new TreeNode({
    html: ["<span>Edge Parent-Child</span>", "<div>Child</div>"],
  });
  parent.addChild(child);
  console.log("\nBefore removal:");
  console.log(parent.visualize());
  const removed = parent.removeChild(child);
  console.log("\nAfter removal:");
  console.log(parent.visualize());
  runner.assert(removed === true, "Child removal should return true");
  runner.assert(
    parent.children.length === 0,
    "Parent should have no children after removal"
  );
});

// Test: Update node HTML elements
runner.test("Update node HTML elements", () => {
  const node = new TreeNode({ html: ["<div>Old</div>"] });
  node.updateHtmlElements(["<div>New</div>"]);
  runner.assert(
    node.nodeData.html.join("") === "<div>New</div>",
    "Update HTML elements failed"
  );
});

// Test: Complex nested tree structure
runner.test("Complex nested tree structure", () => {
  // Create a complex family tree with multiple generations and relationships
  const root = new TreeNode(
    { html: ["<div>Grandparent</div>"] },
    { generation: 1 } // data as second argument
  );
  const parent1 = new TreeNode(
    { html: ["<span>Edge GP-P1</span>", "<div>Parent 1</div>"] },
    { generation: 2 }
  );
  const parent2 = new TreeNode(
    { html: ["<span>Edge GP-P2</span>", "<div>Parent 2</div>"] },
    { generation: 2 }
  );
  const parent3 = new TreeNode(
    { html: ["<span>Edge GP-P3</span>", "<div>Parent 3</div>"] },
    { generation: 2 }
  );
  root.addChild(parent1);
  root.addChild(parent2);
  root.addChild(parent3);
  const child1A = new TreeNode(
    { html: ["<span>Edge P1-C1A</span>", "<div>Child 1A</div>"] },
    { generation: 3 }
  );
  const child1B = new TreeNode(
    { html: ["<span>Edge P1-C1B</span>", "<div>Child 1B</div>"] },
    { generation: 3 }
  );
  parent1.addChild(child1A);
  parent1.addChild(child1B);
  const child2A = new TreeNode(
    { html: ["<span>Edge P2-C2A</span>", "<div>Child 2A</div>"] },
    { generation: 3 }
  );
  parent2.addChild(child2A);
  const greatChild1 = new TreeNode(
    { html: ["<span>Edge C1A-GC1</span>", "<div>Great-Grandchild 1</div>"] },
    { generation: 4 }
  );
  const greatChild2 = new TreeNode(
    { html: ["<span>Edge C1A-GC2</span>", "<div>Great-Grandchild 2</div>"] },
    { generation: 4 }
  );
  child1A.addChild(greatChild1);
  child1A.addChild(greatChild2);
  console.log("\nComplex tree visualization:");
  console.log(root.visualize());
  runner.assert(root.children.length === 3, "Root should have 3 children");
  runner.assert(
    parent1.children.length === 2,
    "Parent 1 should have 2 children"
  );
  runner.assert(parent2.children.length === 1, "Parent 2 should have 1 child");
  runner.assert(
    parent3.children.length === 0,
    "Parent 3 should have no children"
  );
});

// Test: Active Path Rendering
runner.test("Active Path Rendering", () => {
  // Build a tree structure:
  //       Root
  //         └── A (active)
  //               └── A2 (active)
  //                      └── A2b (active)
  const root = new TreeNode({ html: ["<div>Root</div>"] });
  const A = new TreeNode({
    html: ["<span>Edge Root-A</span>", "<div>A</div>"],
  });
  const B = new TreeNode({
    html: ["<span>Edge Root-B</span>", "<div>B</div>"],
  });
  const C = new TreeNode({
    html: ["<span>Edge Root-C</span>", "<div>C</div>"],
  });
  root.addChild(A);
  root.addChild(B);
  root.addChild(C);
  const A1 = new TreeNode({
    html: ["<span>Edge A-A1</span>", "<div>A1</div>"],
  });
  const A2 = new TreeNode({
    html: ["<span>Edge A-A2</span>", "<div>A2</div>"],
  });
  const A3 = new TreeNode({
    html: ["<span>Edge A-A3</span>", "<div>A3</div>"],
  });
  A.addChild(A1);
  A.addChild(A2);
  A.addChild(A3);
  A.setActiveChild(A2);
  const A2a = new TreeNode({
    html: ["<span>Edge A2-A2a</span>", "<div>A2a</div>"],
  });
  const A2b = new TreeNode({
    html: ["<span>Edge A2-A2b</span>", "<div>A2b</div>"],
  });
  const A2c = new TreeNode({
    html: ["<span>Edge A2-A2c</span>", "<div>A2c</div>"],
  });
  A2.addChild(A2a);
  A2.addChild(A2b);
  A2.addChild(A2c);
  A2.setActiveChild(A2b);

  console.log("\nFull tree visualization:");
  console.log(root.visualize());

  console.log("\nActive path visualization:");
  console.log(root.visualizeActivePath());

  const expected =
    "<div>Root</div>" +
    "<span>Edge Root-A</span><div>A</div>" +
    "<span>Edge A-A2</span><div>A2</div>" +
    "<span>Edge A2-A2b</span><div>A2b</div>";
  const activePathHtml = root.renderActivePath();
  runner.assert(
    activePathHtml === expected,
    "Active path HTML did not match expected output"
  );
});

// Test: Active Path Recomputing after Changing Active Child
runner.test("Active Path Recomputing after Changing Active Child", () => {
  // Build a tree:
  //        Root
  //        /   \
  //       A     B
  //     /   \  /  \
  //    A1   A2 B1  B2
  const root = new TreeNode({ html: ["<div>Root</div>"] });
  const A = new TreeNode({
    html: ["<span>Edge Root-A</span>", "<div>A</div>"],
  });
  const B = new TreeNode({
    html: ["<span>Edge Root-B</span>", "<div>B</div>"],
  });
  root.addChild(A);
  root.addChild(B);

  // Under A, add A1 and A2 (A1 is active by default)
  const A1 = new TreeNode({
    html: ["<span>Edge A-A1</span>", "<div>A1</div>"],
  });
  const A2 = new TreeNode({
    html: ["<span>Edge A-A2</span>", "<div>A2</div>"],
  });
  A.addChild(A1);
  A.addChild(A2);

  // Under B, add B1 and B2 (B1 is active by default)
  const B1 = new TreeNode({
    html: ["<span>Edge B-B1</span>", "<div>B1</div>"],
  });
  const B2 = new TreeNode({
    html: ["<span>Edge B-B2</span>", "<div>B2</div>"],
  });
  B.addChild(B1);
  B.addChild(B2);

  console.log("\nInitial full tree visualization:");
  console.log(root.visualize());

  console.log("\nInitial active path visualization:");
  console.log(root.visualizeActivePath());

  // Change A's active child from A1 to A2
  A.setActiveChild(A2);
  console.log("\nActive path after changing A's active child to A2:");
  console.log(root.visualizeActivePath());

  // Change Root's active child from A to B
  root.setActiveChild(B);
  console.log("\nActive path after changing Root's active child to B:");
  console.log(root.visualizeActivePath());

  const expected =
    "<div>Root</div>" +
    "<span>Edge Root-B</span><div>B</div>" +
    "<span>Edge B-B1</span><div>B1</div>";
  const activePath = root.renderActivePath();
  runner.assert(
    activePath === expected,
    "Active path after changing Root's active child does not match expected"
  );
});

// Test: Active Path Recomputing on Deep Tree
runner.test("Active Path Recomputing on Deep Tree", () => {
  // Build a deeper tree:
  //        Root
  //         └── A
  //              └── A1
  //                   ├── A1a (default active)
  //                   └── A1b
  const root = new TreeNode({ html: ["<div>Root</div>"] });
  const A = new TreeNode({
    html: ["<span>Edge Root-A</span>", "<div>A</div>"],
  });
  root.addChild(A);
  const A1 = new TreeNode({
    html: ["<span>Edge A-A1</span>", "<div>A1</div>"],
  });
  A.addChild(A1);
  const A1a = new TreeNode({
    html: ["<span>Edge A1-A1a</span>", "<div>A1a</div>"],
  });
  const A1b = new TreeNode({
    html: ["<span>Edge A1-A1b</span>", "<div>A1b</div>"],
  });
  A1.addChild(A1a);
  A1.addChild(A1b);

  console.log("\nInitial full tree visualization:");
  console.log(root.visualize());

  console.log("\nInitial active path visualization:");
  console.log(root.visualizeActivePath());

  // Change A1's active child from A1a to A1b
  A1.setActiveChild(A1b);
  console.log("\nActive path after changing A1's active child to A1b:");
  console.log(root.visualizeActivePath());

  const expected =
    "<div>Root</div>" +
    "<span>Edge Root-A</span><div>A</div>" +
    "<span>Edge A-A1</span><div>A1</div>" +
    "<span>Edge A1-A1b</span><div>A1b</div>";
  const activePath = root.renderActivePath();
  runner.assert(
    activePath === expected,
    "Active path after changing A1's active child does not match expected"
  );
});

// Test: HTML Path Reconstruction
runner.test("HTML Path Reconstruction", () => {
  // Create a deeply nested tree with multiple branches
  const root = new TreeNode({ html: ['<div class="root">'] });

  // Branch A with deep nesting
  const branchA = new TreeNode({ html: ['<div class="branch-a">'] });
  const a1 = new TreeNode({ html: ['<div class="a1">'] });
  const a1x = new TreeNode({ html: ['<div class="a1x">'] });
  const a1y = new TreeNode({ html: ['<div class="a1y">'] });
  const a1z = new TreeNode({ html: ['<div class="a1z">'] });

  // Branch B with multiple sub-branches
  const branchB = new TreeNode({ html: ['<div class="branch-b">'] });
  const b1 = new TreeNode({ html: ['<div class="b1">'] });
  const b2 = new TreeNode({ html: ['<div class="b2">'] });
  const b1x = new TreeNode({ html: ['<div class="b1x">'] });
  const b1y = new TreeNode({ html: ['<div class="b1y">'] });
  const b2x = new TreeNode({ html: ['<div class="b2x">'] });

  // Branch C with asymmetric depth
  const branchC = new TreeNode({ html: ['<div class="branch-c">'] });
  const c1 = new TreeNode({ html: ['<div class="c1">'] });
  const c2 = new TreeNode({ html: ['<div class="c2">'] });
  const c1x = new TreeNode({ html: ['<div class="c1x">'] });
  const c1x1 = new TreeNode({ html: ['<div class="c1x1">'] });
  const c1x2 = new TreeNode({ html: ['<div class="c1x2">'] });
  const c1x1a = new TreeNode({ html: ['<div class="c1x1a">'] });

  // Build the complex tree structure
  root.addChild(branchA);
  root.addChild(branchB);
  root.addChild(branchC);

  branchA.addChild(a1);
  a1.addChild(a1x);
  a1.addChild(a1y);
  a1y.addChild(a1z);

  branchB.addChild(b1);
  branchB.addChild(b2);
  b1.addChild(b1x);
  b1.addChild(b1y);
  b2.addChild(b2x);

  branchC.addChild(c1);
  branchC.addChild(c2);
  c1.addChild(c1x);
  c1x.addChild(c1x1);
  c1x.addChild(c1x2);
  c1x1.addChild(c1x1a);

  // Print the full tree structure
  console.log("\nComplete tree structure:");
  console.log(root.visualize());

  // Test different paths through the tree
  console.log("\nPath 1 - Deepest A branch:");
  root.setActiveChild(branchA);
  branchA.setActiveChild(a1);
  a1.setActiveChild(a1y);
  a1y.setActiveChild(a1z);
  console.log(root.renderActivePath());

  console.log("\nPath 2 - B branch with b1x:");
  root.setActiveChild(branchB);
  branchB.setActiveChild(b1);
  b1.setActiveChild(b1x);
  console.log(root.renderActivePath());

  console.log("\nPath 3 - Deepest C branch:");
  root.setActiveChild(branchC);
  branchC.setActiveChild(c1);
  c1.setActiveChild(c1x);
  c1x.setActiveChild(c1x1);
  c1x1.setActiveChild(c1x1a);
  console.log(root.renderActivePath());

  // Verify the structure
  const deepestCPath = root.renderActivePath();
  runner.assert(
    deepestCPath.includes("root") &&
      deepestCPath.includes("branch-c") &&
      deepestCPath.includes("c1x1a"),
    "Deepest C path should contain root, branch-c, and c1x1a elements"
  );

  root.setActiveChild(branchB);
  const bPath = root.renderActivePath();
  runner.assert(
    bPath.includes("root") &&
      bPath.includes("branch-b") &&
      bPath.includes("b1x"),
    "B path should contain root, branch-b, and b1x elements"
  );
});

// Test: Accumulated Data Along Active Path
runner.test("Accumulated Data Along Active Path", () => {
  // Create a tree with multiple types of data
  const root = new TreeNode({
    html: ["<div>Root</div>"],
    messages: ["Root message"],
    metadata: [{ level: 1 }],
  });

  const childA = new TreeNode({
    html: ["<span>Edge Root-A</span>", "<div>A</div>"],
    messages: ["A message"],
    metadata: [{ level: 2 }],
  });

  const childB = new TreeNode({
    html: ["<span>Edge A-B</span>", "<div>B</div>"],
    messages: ["B message"],
    metadata: [{ level: 3 }],
  });

  root.addChild(childA);
  childA.addChild(childB);

  // Test accumulation of different data types
  const htmlPath = root.getActivePathData("html");
  runner.assert(
    htmlPath.join("") ===
      "<div>Root</div><span>Edge Root-A</span><div>A</div><span>Edge A-B</span><div>B</div>",
    "HTML path accumulation failed"
  );

  const messagePath = root.getActivePathData("messages");
  runner.assert(
    messagePath.join(" ") === "Root message A message B message",
    "Message path accumulation failed"
  );

  const metadataPath = root.getActivePathData("metadata");
  runner.assert(
    metadataPath.length === 3 &&
      metadataPath[0].level === 1 &&
      metadataPath[1].level === 2 &&
      metadataPath[2].level === 3,
    "Metadata path accumulation failed"
  );

  // Test getting all accumulated data at once
  const allData = root.getAllActivePathData();
  runner.assert(
    Object.keys(allData).length === 3 &&
      allData.html &&
      allData.messages &&
      allData.metadata,
    "Getting all accumulated data failed"
  );
});

// Run all tests
runner.run();

// Export for use in other modules if needed
if (typeof module !== "undefined" && module.exports) {
  module.exports = { TreeNode };
}
