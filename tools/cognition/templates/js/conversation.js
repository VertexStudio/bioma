/**
 * @template T
 * @typedef {Object} Edge
 * @property {string} html - HTML content of the edge
 * @property {T} data - Optional data associated with the edge
 */

/**
 * @template T
 * @class TreeNode
 */
class TreeNode {
  /**
   * @param {string} html - HTML content of the node
   * @param {T} [data] - Optional data associated with the node
   */
  constructor(html, data = undefined) {
    this.html = html;
    this.data = data;
    /** @type {Map<TreeNode<T>, Edge<T>>} */
    this.children = new Map();
    /** @type {TreeNode<T> | null} */
    this.activeChild = null;
  }

  /**
   * Add a child node with an edge.
   * If no active child is set, the first added child becomes active (i.e. the leftmost child).
   * @param {TreeNode<T>} child - Child node to add
   * @param {string} edgeHtml - HTML content for the edge
   * @param {T} [edgeData] - Optional data for the edge
   * @returns {TreeNode<T>} - The added child node
   */
  addChild(child, edgeHtml, edgeData = undefined) {
    this.children.set(child, { html: edgeHtml, data: edgeData });
    // If no active child is set, set this new child as active.
    if (this.activeChild === null) {
      this.activeChild = child;
    }
    return child;
  }

  /**
   * Remove a child node.
   * If the removed child was the active child, update the active child to the leftmost remaining child (or null).
   * @param {TreeNode<T>} child - Child node to remove
   * @returns {boolean} - True if the child was removed
   */
  removeChild(child) {
    const removed = this.children.delete(child);
    if (removed && this.activeChild === child) {
      const remaining = this.getChildren();
      this.activeChild = remaining.length > 0 ? remaining[0] : null;
    }
    return removed;
  }

  /**
   * Get the edge connecting to a child.
   * @param {TreeNode<T>} child - Child node
   * @returns {Edge<T> | undefined} - The edge or undefined if child not found
   */
  getEdge(child) {
    return this.children.get(child);
  }

  /**
   * Get all children nodes.
   * @returns {TreeNode<T>[]} - Array of child nodes
   */
  getChildren() {
    return Array.from(this.children.keys());
  }

  /**
   * Update node's HTML content.
   * @param {string} html - New HTML content
   */
  updateHtml(html) {
    this.html = html;
  }

  /**
   * Update edge HTML content to a specific child.
   * @param {TreeNode<T>} child - Target child node
   * @param {string} html - New HTML content for the edge
   * @returns {boolean} - True if the edge was updated
   */
  updateEdgeHtml(child, html) {
    const edge = this.children.get(child);
    if (edge) {
      edge.html = html;
      return true;
    }
    return false;
  }

  /**
   * Set the active child of this node.
   * @param {TreeNode<T>} child - Child node to set as active
   */
  setActiveChild(child) {
    if (!this.children.has(child)) {
      throw new Error("Child not found in this node.");
    }
    this.activeChild = child;
  }

  /**
   * Get the active child of this node.
   * If no active child is set but there are children, defaults to the leftmost child.
   * @returns {TreeNode<T> | null}
   */
  getActiveChild() {
    if (!this.activeChild && this.children.size > 0) {
      this.activeChild = this.getChildren()[0];
    }
    return this.activeChild;
  }

  /**
   * Recursively render the HTML along the active path.
   * It renders this node's HTML, then the edge HTML to the active child,
   * then recursively the active child's rendered HTML.
   * @returns {string} - Concatenated HTML from this node down its active branch.
   */
  renderActivePath() {
    let result = this.html;
    const active = this.getActiveChild();
    if (active) {
      const edge = this.getEdge(active);
      result += edge.html;
      result += active.renderActivePath();
    }
    return result;
  }

  /**
   * Generate a string visualization of the tree.
   * @param {string} [prefix=''] - Prefix for current line
   * @param {boolean} [isLast=true] - Is this the last child of its parent
   * @returns {string} - String representation of the tree
   */
  visualize(prefix = "", isLast = true) {
    let result =
      prefix + (prefix ? (isLast ? "└── " : "├── ") : "") + this.html + "\n";

    const children = this.getChildren();
    children.forEach((child, index) => {
      const edge = this.getEdge(child);
      const newPrefix = prefix + (isLast ? "    " : "│   ");
      const isLastChild = index === children.length - 1;

      // Add edge visualization
      result += newPrefix + (isLastChild ? "└─┤ " : "├─┤ ") + edge.html + "\n";
      result += child.visualize(newPrefix, isLastChild);
    });

    return result;
  }

  /**
   * Generate a string visualization of the active path.
   * @param {string} [prefix=''] - Prefix for current line
   * @returns {string} - String representation of the active path
   */
  visualizeActivePath(prefix = "") {
    let result = prefix + (prefix ? "└── " : "") + this.html + "\n";

    const activeChild = this.getActiveChild();
    if (activeChild) {
      const edge = this.getEdge(activeChild);
      const newPrefix = prefix + "    ";

      // Add edge visualization
      result += newPrefix + "└─┤ " + edge.html + "\n";
      result += activeChild.visualizeActivePath(newPrefix);
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

// Run tests
const runner = new TestRunner();

runner.test("Create node with HTML content", () => {
  const node = new TreeNode("<div>Test</div>");
  console.log("\nTree visualization:");
  console.log(node.visualize());
  runner.assert(node.html === "<div>Test</div>", "Node HTML content mismatch");
  runner.assert(node.children.size === 0, "New node should have no children");
});

runner.test("Add child with edge", () => {
  const parent = new TreeNode("<div>Parent</div>");
  const child = new TreeNode("<div>Child</div>");
  parent.addChild(child, "<line>Connection</line>");

  console.log("\nTree visualization:");
  console.log(parent.visualize());
  runner.assert(parent.children.size === 1, "Parent should have one child");
  runner.assert(
    parent.getEdge(child).html === "<line>Connection</line>",
    "Edge HTML content mismatch"
  );
});

runner.test("Remove child", () => {
  const parent = new TreeNode("<div>Parent</div>");
  const child = new TreeNode("<div>Child</div>");
  parent.addChild(child, "<line>Connection</line>");

  console.log("\nBefore removal:");
  console.log(parent.visualize());

  const removed = parent.removeChild(child);

  console.log("\nAfter removal:");
  console.log(parent.visualize());
  runner.assert(removed === true, "Child removal should return true");
  runner.assert(
    parent.children.size === 0,
    "Parent should have no children after removal"
  );
});

runner.test("Update node and edge HTML", () => {
  const parent = new TreeNode("<div>Parent</div>");
  const child = new TreeNode("<div>Child</div>");
  parent.addChild(child, "<line>Connection</line>");

  console.log("\nBefore updates:");
  console.log(parent.visualize());

  parent.updateHtml("<div>Updated Parent</div>");
  parent.updateEdgeHtml(child, "<line>Updated Connection</line>");

  console.log("\nAfter updates:");
  console.log(parent.visualize());
  runner.assert(
    parent.html === "<div>Updated Parent</div>",
    "Node HTML update failed"
  );
  runner.assert(
    parent.getEdge(child).html === "<line>Updated Connection</line>",
    "Edge HTML update failed"
  );
});

runner.test("Generic type support", () => {
  const parent = new TreeNode("<div>Parent</div>", { id: 1 });
  const child = new TreeNode("<div>Child</div>", { id: 2 });
  parent.addChild(child, "<line>Connection</line>", { weight: 10 });

  console.log("\nTree visualization (with data):");
  console.log(parent.visualize());
  runner.assert(parent.data.id === 1, "Node data mismatch");
  runner.assert(parent.getEdge(child).data.weight === 10, "Edge data mismatch");
});

runner.test("Complex nested tree structure", () => {
  // Create a complex family tree with multiple generations and relationships
  const root = new TreeNode("<div>Grandparent</div>", { generation: 1 });

  // First generation children
  const parent1 = new TreeNode("<div>Parent 1</div>", { generation: 2 });
  const parent2 = new TreeNode("<div>Parent 2</div>", { generation: 2 });
  const parent3 = new TreeNode("<div>Parent 3</div>", { generation: 2 });

  root.addChild(parent1, "<line>First child</line>", { relation: "eldest" });
  root.addChild(parent2, "<line>Second child</line>", { relation: "middle" });
  root.addChild(parent3, "<line>Third child</line>", { relation: "youngest" });

  // Second generation - Parent 1's children
  const child1A = new TreeNode("<div>Child 1A</div>", { generation: 3 });
  const child1B = new TreeNode("<div>Child 1B</div>", { generation: 3 });
  parent1.addChild(child1A, "<line>First grandchild</line>", {
    branch: "left",
  });
  parent1.addChild(child1B, "<line>Second grandchild</line>", {
    branch: "left",
  });

  // Second generation - Parent 2's child
  const child2A = new TreeNode("<div>Child 2A</div>", { generation: 3 });
  parent2.addChild(child2A, "<line>Third grandchild</line>", {
    branch: "middle",
  });

  // Third generation - Child 1A's children
  const greatChild1 = new TreeNode("<div>Great-Grandchild 1</div>", {
    generation: 4,
  });
  const greatChild2 = new TreeNode("<div>Great-Grandchild 2</div>", {
    generation: 4,
  });
  child1A.addChild(greatChild1, "<line>First great-grandchild</line>", {
    branch: "leftmost",
  });
  child1A.addChild(greatChild2, "<line>Second great-grandchild</line>", {
    branch: "leftmost",
  });

  console.log("\nComplex tree visualization:");
  console.log(root.visualize());

  // Assertions to verify the structure
  runner.assert(root.getChildren().length === 3, "Root should have 3 children");
  runner.assert(
    parent1.getChildren().length === 2,
    "Parent 1 should have 2 children"
  );
  runner.assert(
    parent2.getChildren().length === 1,
    "Parent 2 should have 1 child"
  );
  runner.assert(
    parent3.getChildren().length === 0,
    "Parent 3 should have no children"
  );
  runner.assert(
    child1A.getChildren().length === 2,
    "Child 1A should have 2 children"
  );

  // Verify data and relationships
  runner.assert(root.data.generation === 1, "Root should be generation 1");
  runner.assert(
    parent1.data.generation === 2,
    "Parent 1 should be generation 2"
  );
  runner.assert(
    child1A.data.generation === 3,
    "Child 1A should be generation 3"
  );
  runner.assert(
    greatChild1.data.generation === 4,
    "Great-Grandchild 1 should be generation 4"
  );

  // Verify edge data
  runner.assert(
    root.getEdge(parent1).data.relation === "eldest",
    "Parent 1 should be eldest child"
  );
  runner.assert(
    child1A.getEdge(greatChild1).data.branch === "leftmost",
    "Great-Grandchild 1 should be on leftmost branch"
  );
});

runner.test("Active Path Rendering", () => {
  // Build a tree structure similar to the example:
  //
  //       Root
  //         └── A (active)
  //               └── A2 (active)
  //                      └── A2b (active)
  // Other children exist but are not on the active path.
  const root = new TreeNode("<div>Root</div>");
  const A = new TreeNode("<div>A</div>");
  const B = new TreeNode("<div>B</div>");
  const C = new TreeNode("<div>C</div>");
  root.addChild(A, "<span>Edge Root-A</span>");
  root.addChild(B, "<span>Edge Root-B</span>");
  root.addChild(C, "<span>Edge Root-C</span>");

  // Under A, add A1, A2, A3. (A is active by default because it is the first child of root)
  const A1 = new TreeNode("<div>A1</div>");
  const A2 = new TreeNode("<div>A2</div>");
  const A3 = new TreeNode("<div>A3</div>");
  A.addChild(A1, "<span>Edge A-A1</span>");
  A.addChild(A2, "<span>Edge A-A2</span>");
  A.addChild(A3, "<span>Edge A-A3</span>");
  // Set A2 explicitly as active
  A.setActiveChild(A2);

  // Under A2, add A2a, A2b, A2c.
  const A2a = new TreeNode("<div>A2a</div>");
  const A2b = new TreeNode("<div>A2b</div>");
  const A2c = new TreeNode("<div>A2c</div>");
  A2.addChild(A2a, "<span>Edge A2-A2a</span>");
  A2.addChild(A2b, "<span>Edge A2-A2b</span>");
  A2.addChild(A2c, "<span>Edge A2-A2c</span>");
  // Set A2b explicitly as active
  A2.setActiveChild(A2b);

  // The expected active path HTML:
  // Root html + edge Root-A + A html + edge A-A2 + A2 html + edge A2-A2b + A2b html
  const expected =
    "<div>Root</div>" +
    "<span>Edge Root-A</span>" +
    "<div>A</div>" +
    "<span>Edge A-A2</span>" +
    "<div>A2</div>" +
    "<span>Edge A2-A2b</span>" +
    "<div>A2b</div>";

  const activePathHtml = root.renderActivePath();
  console.log("\nActive path HTML:");
  console.log(activePathHtml);
  runner.assert(
    activePathHtml === expected,
    "Active path HTML did not match expected output"
  );

  console.log("\nActive path visualization:");
  console.log(root.visualizeActivePath());
});

// Run all tests
runner.run();

// Export for use in other modules
if (typeof module !== "undefined" && module.exports) {
  module.exports = { TreeNode };
}
