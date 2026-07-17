// Selection state for the activities list. Delegated from document rather than
// bound per row: the list is swapped wholesale by htmx on every sse:refresh and
// every sort/filter change, and handlers bound to the old nodes would die with
// them.
(function () {
    "use strict";

    function panelOf(el) {
        return el.closest(".activity-panel");
    }

    function selectedInputs(panel) {
        return panel.querySelectorAll(".activsync-select-input:checked");
    }

    function selectableInputs(panel) {
        return panel.querySelectorAll(".activsync-select-input");
    }

    // The input is the only place selected-ness lives: the CSS reads it with
    // :has(:checked) and hx-include posts it. Nothing here mirrors it onto the
    // row, so there is nothing to keep in sync.
    // Only the bar's own labels need updating when it changes.
    function refresh(panel) {
        var selected = selectedInputs(panel).length;
        var selectable = selectableInputs(panel).length;

        var count = panel.querySelector(".selection-count");
        if (count) {
            count.textContent = selected + " selected";
        }

        var publish = panel.querySelector(".publish-selected-button");
        if (publish) {
            var label = publish.querySelector(".button-label");
            if (label) label.textContent = selected ? "Publish " + selected : "Publish";
            publish.disabled = selected === 0 || publish.dataset.connectionsBroken === "true" ||
                publish.getAttribute("aria-busy") === "true" || publish.classList.contains("htmx-request");
        }

        var all = panel.querySelector(".select-all-input");
        if (all) {
            all.checked = selectable > 0 && selected === selectable;
            all.indeterminate = selected > 0 && selected < selectable;
        }
    }

    function exitSelectMode(panel) {
        panel.classList.remove("is-selecting");
        var toggle = panel.querySelector(".select-multiple-button");
        if (toggle) {
            toggle.setAttribute("aria-pressed", "false");
            toggle.textContent = "Select multiple";
        }
        selectableInputs(panel).forEach(function (input) { input.checked = false; });
        refresh(panel);
    }

    function enterSelectMode(panel) {
        panel.classList.add("is-selecting");
        var toggle = panel.querySelector(".select-multiple-button");
        if (toggle) {
            toggle.setAttribute("aria-pressed", "true");
            toggle.textContent = "Done";
        }
        refresh(panel);
    }

    document.addEventListener("click", function (event) {
        var toggle = event.target.closest(".select-multiple-button");
        if (toggle) {
            var panel = panelOf(toggle);
            if (panel.classList.contains("is-selecting")) exitSelectMode(panel);
            else enterSelectMode(panel);
            return;
        }

        var row = event.target.closest(".activity-row");
        if (!row) return;
        var rowPanel = panelOf(row);
        if (!rowPanel || !rowPanel.classList.contains("is-selecting")) return;

        // The row is the hit target, but it still contains links and the
        // checkbox's own label — let those behave normally. Without `label`
        // here, a click on the box would toggle twice and land back where it
        // started: once natively, once from this handler.
        if (event.target.closest("a, button, input, select, textarea, dialog, label")) return;

        var input = row.querySelector(".activsync-select-input");
        if (!input) return;              // published/excluded rows are inert
        input.checked = !input.checked;
        refresh(rowPanel);
    });

    // Both checkboxes are reached by click, by keyboard, and by their label, and
    // only `change` catches all three — a click handler misses the label text
    // and the spacebar.
    document.addEventListener("change", function (event) {
        var all = event.target.closest(".select-all-input");
        if (all) {
            var allPanel = panelOf(all);
            selectableInputs(allPanel).forEach(function (input) { input.checked = all.checked; });
            refresh(allPanel);
            return;
        }

        var input = event.target.closest(".activsync-select-input");
        if (input) refresh(panelOf(input));
    });

    document.addEventListener("keydown", function (event) {
        if (event.key !== "Escape") return;
        var panel = document.querySelector(".activity-panel.is-selecting");
        if (panel && !document.querySelector("dialog[open]")) exitSelectMode(panel);
    });

    // htmx swaps the panel out from under us; the fresh markup renders at rest,
    // so the mode is not preserved across a refresh by design.
    document.addEventListener("htmx:afterSwap", function (event) {
        var panel = event.target.closest && event.target.closest(".activity-panel");
        if (panel) refresh(panel);
    });
})();
